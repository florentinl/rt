use std::{collections::HashMap, ffi::CString, fs, path::Path};

use indexmap::{IndexMap, IndexSet};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyAnyMethods, PyDict, PyIterator, PyModule, PyString, PyStringMethods};

use crate::{
    config_provider::{ConfigProvider, LoadedConfig, ProviderServices, ProviderVenvNode},
    error::{RtError, RtResult},
};

use super::fake_ruamel_yaml;

pub struct Pyo3ConfigProvider;

impl ConfigProvider for Pyo3ConfigProvider {
    fn load(riotfile_path: &Path) -> RtResult<LoadedConfig> {
        let project_path = riotfile_path.parent().ok_or_else(|| {
            RtError::message("error: could not determine riotfile parent directory")
        })?;

        Python::initialize();
        Python::attach(|py| {
            py.import("gc")
                .and_then(|gc| gc.call_method0("disable"))
                .map_err(|err| {
                    RtError::message(format!(
                        "error: could not configure embedded Python garbage collector: {err}"
                    ))
                })?;

            let root = load_riotfile(py, riotfile_path)?;
            let services = get_services(py, project_path)?;

            Ok(LoadedConfig { root, services })
        })
    }
}

#[derive(Clone)]
#[pyclass(from_py_object, name = "Venv", module = "riot")]
struct PyVenv {
    pub name: Option<String>,
    pub command: Option<String>,
    pub pys: Vec<String>,
    pkgs: IndexMap<String, Vec<String>>,
    env: IndexMap<String, Vec<String>>,
    pub create: Option<bool>,
    pub skip_dev_install: Option<bool>,
    pub venvs: Vec<Self>,
}

#[pymethods]
impl PyVenv {
    #[new]
    #[allow(clippy::too_many_arguments)]
    #[pyo3(
        signature = (name=None, command=None, pys=None, pkgs=None, env=None, venvs=None, create=None, skip_dev_install=None)
    )]
    fn new(
        py: Python<'_>,
        name: Option<String>,
        command: Option<String>,
        pys: Option<Py<PyAny>>,
        pkgs: Option<Py<PyAny>>,
        env: Option<Py<PyAny>>,
        venvs: Option<Py<PyAny>>,
        create: Option<bool>,
        skip_dev_install: Option<bool>,
    ) -> PyResult<Self> {
        let venvs = venvs
            .map(|value| value.bind(py).extract::<Vec<Self>>())
            .transpose()?
            .unwrap_or_default();
        Ok(Self {
            name,
            command,
            pys: parse_pys(py, pys)?,
            pkgs: parse_dict_to_vec_map(py, pkgs)?,
            env: parse_dict_to_vec_map(py, env)?,
            create,
            skip_dev_install,
            venvs,
        })
    }
}

impl From<PyVenv> for ProviderVenvNode {
    fn from(value: PyVenv) -> Self {
        Self {
            name: value.name,
            command: value.command,
            pys: value.pys,
            pkgs: value.pkgs,
            env: value.env,
            create: value.create,
            skip_dev_install: value.skip_dev_install,
            venvs: value.venvs.into_iter().map(Self::from).collect(),
        }
    }
}

fn load_riotfile(py: Python<'_>, path: &Path) -> RtResult<ProviderVenvNode> {
    let source = fs::read_to_string(path)
        .map_err(|err| RtError::message(format!("error: failed to read riotfile: {err}")))?;

    let source_cstr = CString::new(source)
        .map_err(|err| RtError::message(format!("error: invalid riotfile content: {err}")))?;

    let module_name = CString::new(
        path.file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("riotfile"),
    )
    .map_err(|err| RtError::message(format!("error: invalid module name: {err}")))?;

    let path_cstr = CString::new(path.to_string_lossy().into_owned())
        .map_err(|err| RtError::message(format!("error: invalid riotfile path: {err}")))?;

    load_fake_riot_module(py).map_err(|err| {
        RtError::message(format!(
            "error: could not load the `riot` module in the interpreter: {err}"
        ))
    })?;

    let module = PyModule::from_code(
        py,
        source_cstr.as_c_str(),
        path_cstr.as_c_str(),
        module_name.as_c_str(),
    )
    .map_err(|err| {
        RtError::message(format!(
            "error: could not load riotfile.py in the python interpreter: {err}"
        ))
    })?;

    let venv_obj = module
        .getattr("venv")
        .map_err(|_| RtError::message("error: riotfile does not define a `venv` variable"))?;

    let venv: PyVenv = venv_obj
        .extract()
        .map_err(|err| RtError::message(format!("error: `venv` variable is invalid: {err}")))?;

    Ok(venv.into())
}

fn load_fake_riot_module(py: Python<'_>) -> PyResult<()> {
    let riot_module = PyModule::new(py, "riot")?;
    riot_module.add_class::<PyVenv>()?;

    let sys = py.import("sys")?;
    let modules = sys.getattr("modules")?;
    let modules: &Bound<'_, PyDict> = modules.cast()?;
    modules.set_item("riot", riot_module)?;

    Ok(())
}

fn get_services(py: Python<'_>, project_path: &Path) -> RtResult<Option<ProviderServices>> {
    let python_code = format!(
        r#"
import sys
sys.path.insert(0, r"{}")

from tests.suitespec import SUITESPEC
result = {{ k.split("::")[-1]: (v.get("services", []) + ["testagent"] if v.get("snapshot") else []) for k, v in SUITESPEC["suites"].items() if v.get("services") or v.get("snapshot") }}
"#,
        project_path.to_string_lossy()
    );

    let Some(code_cstr) = CString::new(python_code).ok() else {
        return Ok(None);
    };
    let module_name = c"_get_service";
    let filename = c"get_services";

    load_fake_ruamel_module(py).map_err(|err| {
        RtError::message(format!(
            "error: could not properly load ruamel.yaml module: {err}"
        ))
    })?;

    let result: Result<HashMap<String, Vec<String>>, PyErr> =
        PyModule::from_code(py, code_cstr.as_c_str(), filename, module_name)
            .and_then(|module| module.getattr("result")?.extract());

    Ok(result.ok())
}

fn load_fake_ruamel_module(py: Python<'_>) -> PyResult<()> {
    let sys = py.import("sys")?;
    let modules = sys.getattr("modules")?;
    let modules: &Bound<'_, PyDict> = modules.cast()?;
    let yaml_module = fake_ruamel_yaml::get_fake_ruamel_yaml(py)?;
    let ruamel_module = PyModule::new(py, "ruamel")?;

    ruamel_module.add("yaml", &yaml_module)?;
    modules.set_item("ruamel", &ruamel_module)?;
    modules.set_item("ruamel.yaml", yaml_module)?;

    Ok(())
}

fn parse_pys(py: Python<'_>, pys: Option<Py<PyAny>>) -> PyResult<Vec<String>> {
    let versions = pys
        .map(|obj| extract_str_list(obj.bind(py)))
        .transpose()?
        .unwrap_or_default();
    Ok(normalize_pys(versions))
}

fn parse_dict_to_vec_map(
    py: Python<'_>,
    obj: Option<Py<PyAny>>,
) -> PyResult<IndexMap<String, Vec<String>>> {
    let mut map = IndexMap::new();
    let Some(obj) = obj else {
        return Ok(map);
    };

    let bound = obj.bind(py);
    if bound.is_none() {
        return Ok(map);
    }

    let dict = bound.cast::<PyDict>()?;
    for (key, value) in dict {
        if value.is_none() {
            continue;
        }
        let name: String = key.extract()?;
        let values = dedup_preserving_order(extract_str_list(value.as_any())?);
        map.insert(name, values);
    }

    Ok(map)
}

fn extract_str_list(obj: &Bound<'_, PyAny>) -> PyResult<Vec<String>> {
    if obj.is_none() {
        return Ok(Vec::new());
    }

    if let Ok(value) = obj.cast::<PyString>() {
        return Ok(vec![value.extract::<String>()?]);
    }

    if let Ok(iter) = PyIterator::from_object(obj.as_any()) {
        let mut values = Vec::new();
        for item in iter {
            let item: Bound<'_, PyAny> = item?;
            if item.is_none() {
                continue;
            }
            values.push(py_stringify(&item)?);
        }
        return Ok(values);
    }

    Ok(vec![py_stringify(obj)?])
}

fn py_stringify(obj: &Bound<'_, PyAny>) -> PyResult<String> {
    Ok(obj.str()?.to_str()?.to_string())
}

fn dedup_preserving_order(values: Vec<String>) -> Vec<String> {
    let mut seen = IndexSet::new();
    values
        .into_iter()
        .filter_map(|value| {
            if seen.insert(value.clone()) {
                Some(value)
            } else {
                None
            }
        })
        .collect()
}

fn normalize_pys(mut versions: Vec<String>) -> Vec<String> {
    versions.retain(|value| !value.is_empty());
    versions.sort_by(|left, right| compare_python_versions(left, right));
    versions.dedup();
    versions
}

fn compare_python_versions(lhs: &str, rhs: &str) -> std::cmp::Ordering {
    match (parse_version_components(lhs), parse_version_components(rhs)) {
        (Some(mut left), Some(mut right)) => {
            let max_len = left.len().max(right.len());
            left.resize(max_len, 0);
            right.resize(max_len, 0);
            for (l, r) in left.iter().zip(right.iter()) {
                let ord = l.cmp(r);
                if ord != std::cmp::Ordering::Equal {
                    return ord;
                }
            }
            std::cmp::Ordering::Equal
        }
        _ => lhs.cmp(rhs),
    }
}

fn parse_version_components(version: &str) -> Option<Vec<u32>> {
    if version.is_empty() {
        return Some(vec![]);
    }

    let mut components = Vec::new();
    for part in version.split('.') {
        components.push(part.parse::<u32>().ok()?);
    }
    Some(components)
}
