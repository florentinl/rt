use std::{ffi::CString, fs, path::Path};

use indexmap::IndexMap;
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyAnyMethods, PyDict, PyIterator, PyModule, PyStringMethods};

use crate::{
    config_provider::{ConfigProvider, LoadedConfig, ProviderVenvNode, pyo3::get_suites},
    error::{RtError, RtResult},
};

/// Load the effective configuration produced by Riot itself.
///
/// This provider deliberately stops at Riot's in-memory configuration API: it
/// constructs `Venv` objects and iterates `Venv.instances()`, but never asks an
/// interpreter for its path or version and never calls Riot's preparation or
/// requirements APIs. Environment creation and dependency installation remain
/// exclusively owned by rt.
pub struct RiotConfigProvider;

impl ConfigProvider for RiotConfigProvider {
    fn load(riotfile_path: &Path) -> RtResult<LoadedConfig> {
        let project_path = riotfile_path.parent().ok_or_else(|| {
            RtError::message("error: could not determine riotfile parent directory")
        })?;

        Python::attach(|py| {
            py.import("gc")?.call_method0("disable")?;

            let root = load_effective_instances(py, riotfile_path)?;
            let suites = get_suites(py, project_path);

            Ok(LoadedConfig { root, suites })
        })
    }
}

fn load_effective_instances(py: Python<'_>, path: &Path) -> RtResult<ProviderVenvNode> {
    // Fail before executing project code if the pinned Riot runtime is not
    // available. Importing Riot itself does not discover interpreters or touch
    // its environment root.
    py.import("riot")?;

    let source = fs::read_to_string(path)?;
    let source_cstr = CString::new(source)?;
    let module_name = CString::new(
        path.file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("riotfile"),
    )?;
    let path_cstr = CString::new(path.to_string_lossy().into_owned())?;

    let module = PyModule::from_code(
        py,
        source_cstr.as_c_str(),
        path_cstr.as_c_str(),
        module_name.as_c_str(),
    )?;

    let venv = module.getattr("venv")?;
    let instances = venv.call_method0("instances")?;
    let instances = PyIterator::from_object(instances.as_any())?;

    let mut leaves = Vec::new();
    for instance in instances {
        // Snapshot each yield before advancing the generator. Riot shares some
        // intermediate dictionaries between sibling instances, while project
        // hooks are allowed to mutate the yielded object.
        leaves.push(extract_instance(&instance?)?);
    }

    Ok(ProviderVenvNode {
        venvs: leaves,
        ..ProviderVenvNode::default()
    })
}

fn extract_instance(instance: &Bound<'_, PyAny>) -> PyResult<ProviderVenvNode> {
    let chain = instance_chain(instance)?;
    let pkgs = extract_full_packages(&chain)?;
    let env = extract_mapping(&instance.getattr("env")?)?;

    let py = instance.getattr("py")?;
    let pys = if py.is_none() {
        Vec::new()
    } else {
        vec![py.getattr("_hint")?.str()?.to_str()?.to_owned()]
    };

    let (create, skip_dev_install) = effective_creation(&chain)?;

    Ok(ProviderVenvNode {
        name: extract_optional_string(instance, "name")?,
        command: extract_optional_string(instance, "command")?,
        pys,
        pkgs,
        env,
        create: Some(create),
        skip_dev_install: Some(skip_dev_install),
        venvs: Vec::new(),
    })
}

/// Return the instance ancestry in leaf-to-root order.
fn instance_chain<'py>(instance: &Bound<'py, PyAny>) -> PyResult<Vec<Bound<'py, PyAny>>> {
    let mut chain = Vec::new();
    let mut current = Some(instance.clone());

    while let Some(item) = current {
        chain.push(item.clone());
        let parent = item.getattr("parent")?;
        current = (!parent.is_none()).then_some(parent);
    }

    Ok(chain)
}

fn extract_full_packages(
    leaf_to_root: &[Bound<'_, PyAny>],
) -> PyResult<IndexMap<String, Vec<String>>> {
    let mut packages = IndexMap::new();

    // Riot resolves package inheritance root-first, with child declarations
    // replacing packages of the same name.
    for instance in leaf_to_root.iter().rev() {
        let declared = extract_mapping(&instance.getattr("pkgs")?)?;
        packages.extend(declared);
    }

    Ok(packages)
}

fn extract_mapping(value: &Bound<'_, PyAny>) -> PyResult<IndexMap<String, Vec<String>>> {
    let dict: &Bound<'_, PyDict> = value.cast()?;
    let mut result = IndexMap::with_capacity(dict.len());

    for (key, value) in dict {
        if value.is_none() {
            continue;
        }
        result.insert(key.extract()?, vec![value.str()?.to_str()?.to_owned()]);
    }

    Ok(result)
}

fn extract_optional_string(instance: &Bound<'_, PyAny>, name: &str) -> PyResult<Option<String>> {
    let value = instance.getattr(name)?;
    if value.is_none() {
        Ok(None)
    } else {
        value.extract().map(Some)
    }
}

/// Match Riot's materialization boundary: the nearest created ancestor owns
/// whether the project is installed into that environment.
fn effective_creation(leaf_to_root: &[Bound<'_, PyAny>]) -> PyResult<(bool, bool)> {
    for instance in leaf_to_root {
        if instance.getattr("created")?.extract::<bool>()? {
            let skip_dev_install = instance
                .getattr("venv")?
                .getattr("skip_dev_install")?
                .extract()?;
            return Ok((true, skip_dev_install));
        }
    }

    Ok((false, false))
}

#[cfg(test)]
mod tests {
    use std::{env, fs, path::PathBuf, sync::Once};

    use pyo3::{Python, types::PyAnyMethods};
    use tempfile::tempdir;

    use super::RiotConfigProvider;
    use crate::{
        config_provider::{ConfigProvider, Pyo3ConfigProvider},
        venv::load_context,
    };

    static PYTHON_INIT: Once = Once::new();

    fn initialize_python() {
        PYTHON_INIT.call_once(|| {
            Python::initialize();

            // An embedded interpreter does not automatically add the active
            // uv environment's site-packages to sys.path like a Python-launched
            // extension does. Mirror that path setup for Rust unit tests.
            let Ok(virtual_env) = env::var("VIRTUAL_ENV") else {
                return;
            };
            Python::attach(|py| {
                let version = py.version_info();
                let site_packages = if cfg!(windows) {
                    PathBuf::from(&virtual_env).join("Lib/site-packages")
                } else {
                    PathBuf::from(&virtual_env).join(format!(
                        "lib/python{}.{}/site-packages",
                        version.major, version.minor
                    ))
                };
                py.import("site")
                    .unwrap()
                    .call_method1("addsitedir", (site_packages,))
                    .unwrap();
            });
        });
    }

    const RIOTFILE: &str = r#"
from packaging.version import Version
from riot import Venv
from riot.interpreter import Interpreter
from riot.venv import VenvInstance


def forbidden_materialization(*args, **kwargs):
    raise AssertionError("Riot attempted to materialize an environment while parsing configuration")


Interpreter.path = forbidden_materialization
Interpreter.version = forbidden_materialization
Interpreter.venv_path = property(forbidden_materialization)
VenvInstance.prepare = forbidden_materialization
VenvInstance.requirements = property(forbidden_materialization)

venv = Venv(
    pkgs={"base": "==1"},
    env={"PARENT": "yes"},
    venvs=[
        Venv(
            name="demo",
            command="pytest {cmdargs} tests/demo",
            pys=["3.11", "3.12"],
            pkgs={"child": ["==1", "==2"]},
        ),
    ],
)

original_instances = venv.instances


def configured_instances(parent_inst=None):
    for inst in original_instances(parent_inst):
        # Detach from Riot's sibling-shared environment before mutating it.
        inst.env = dict(inst.env or {})
        inst.env["INSTANCE_HASH"] = inst.short_hash
        inst.env["PYTHON_HINT"] = inst.py._hint
        inst.env["MODERN"] = str(Version(inst.py._hint) >= Version("3.12"))
        yield inst


venv.instances = configured_instances
"#;

    #[test]
    fn loads_effective_instances_without_materializing_riot_environments() {
        initialize_python();
        let project = tempdir().unwrap();
        let riotfile = project.path().join("riotfile.py");
        fs::write(&riotfile, RIOTFILE).unwrap();

        let loaded = RiotConfigProvider::load(&riotfile).unwrap();

        assert_eq!(loaded.root.venvs.len(), 4);
        assert!(!project.path().join(".riot").exists());
        for leaf in &loaded.root.venvs {
            assert_eq!(leaf.name.as_deref(), Some("demo"));
            assert_eq!(leaf.pkgs.get("base").unwrap(), &["==1"]);
            assert_eq!(leaf.env.get("PARENT").unwrap(), &["yes"]);
        }
    }

    #[test]
    fn preserves_instance_hooks_and_riot_hashes() {
        initialize_python();
        let project = tempdir().unwrap();
        let riotfile = project.path().join("riotfile.py");
        fs::write(&riotfile, RIOTFILE).unwrap();

        let venvs = load_context::<RiotConfigProvider>(&riotfile, None).unwrap();

        assert_eq!(venvs.len(), 4);
        for (hash, venv) in venvs {
            assert_eq!(venv.execution_contexts.len(), 1);
            let env = &venv.execution_contexts[0].env;
            assert_eq!(env.get("INSTANCE_HASH"), Some(&hash));
            assert_eq!(env.get("PYTHON_HINT"), Some(&venv.python));
            assert_eq!(
                env.get("MODERN"),
                Some(&if venv.python == "3.12" {
                    "True".to_owned()
                } else {
                    "False".to_owned()
                })
            );
        }
    }

    #[test]
    fn legacy_constructor_provider_remains_available() {
        initialize_python();
        let project = tempdir().unwrap();
        let riotfile = project.path().join("riotfile.py");
        fs::write(
            &riotfile,
            r#"
from riot import Venv

venv = Venv(
    name="legacy",
    command="pytest tests/legacy",
    pys=["3.12"],
    pkgs={"pytest": "==8"},
)
"#,
        )
        .unwrap();

        let loaded = Pyo3ConfigProvider::load(&riotfile).unwrap();

        assert_eq!(loaded.root.name.as_deref(), Some("legacy"));
        assert_eq!(loaded.root.pys, ["3.12"]);
        assert_eq!(loaded.root.pkgs.get("pytest").unwrap(), &["==8"]);
    }
}
