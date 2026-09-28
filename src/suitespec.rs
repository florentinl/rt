use std::{collections::HashSet, path::Path};

use indexmap::{IndexMap, IndexSet};
use pyo3::{prelude::*, types::PyDict};

use crate::{
    error::{RtError, RtResult},
    venv::{Suite, TestEnvironment, TestRun, load_resolved_packages},
};

/// Load dd-trace-py's normalized suitespec model without reproducing its YAML semantics.
#[allow(clippy::too_many_lines)]
pub fn load(project_root: &Path) -> RtResult<IndexMap<String, Suite>> {
    Python::attach(|py| {
        py.import("sys")?
            .getattr("path")?
            .call_method1("insert", (0, project_root.to_string_lossy().as_ref()))?;
        let module = py.import("tests.suitespec")?;
        let configs: Bound<'_, PyDict> = module.call_method0("get_suites")?.cast_into()?;
        let kwargs = PyDict::new(py);
        kwargs.set_item("nightly", false)?;
        let matrices: Bound<'_, PyDict> = module
            .call_method("get_test_environments", (), Some(&kwargs))?
            .cast_into()?;

        let mut suites = IndexMap::with_capacity(matrices.len());
        for (suite_name, matrix) in matrices {
            let id: String = suite_name.extract()?;
            let config: Bound<'_, PyDict> = configs
                .get_item(&id)?
                .ok_or_else(|| pyo3::exceptions::PyKeyError::new_err(id.clone()))?
                .cast_into()?;
            let mut patterns: Vec<String> = module
                .call_method1("get_patterns", (&id,))?
                .extract::<HashSet<String>>()?
                .into_iter()
                .collect();
            patterns.sort();
            let source_files = patterns
                .iter()
                .filter(|pattern| pattern.ends_with("suitespec.yml"))
                .cloned()
                .collect();
            let mut services = config
                .get_item("services")?
                .map(|value| value.extract::<Vec<String>>())
                .transpose()?
                .unwrap_or_default()
                .into_iter()
                .collect::<IndexSet<_>>();
            if config
                .get_item("snapshot")?
                .map(|value| value.extract::<bool>())
                .transpose()?
                .unwrap_or(false)
            {
                services.insert("testagent".to_owned());
            }
            let services: Vec<_> = services.into_iter().collect();
            let mut suite_env = config
                .get_item("env")?
                .as_ref()
                .map(extract_string_mapping)
                .transpose()?
                .unwrap_or_default();
            localize_service_hosts(&mut suite_env, &services);

            let mut environments = Vec::new();
            for environment in matrix.try_iter()? {
                let environment = environment?;
                let hash: String = environment.getattr("hash")?.extract()?;
                let lockfile: String = environment.getattr("lockfile")?.str()?.to_str()?.to_owned();
                let direct_dependencies: Vec<String> =
                    environment.getattr("direct_dependencies")?.extract()?;
                let resolved_pkgs = load_resolved_packages(&project_root.join(&lockfile));
                let display_pkgs =
                    crate::venv::build_display_packages(&direct_dependencies, &resolved_pkgs);
                let mut runs = Vec::new();
                for (index, run) in environment.getattr("runs")?.try_iter()?.enumerate() {
                    let run = run?;
                    runs.push(TestRun {
                        id: format!("{hash}@{index}"),
                        index,
                        command: run.getattr("command")?.extract()?,
                        env: extract_string_mapping(&run.getattr("environment")?)?,
                    });
                }
                environments.push(TestEnvironment {
                    suite: id.clone(),
                    hash,
                    name: environment.getattr("name")?.extract()?,
                    integration_name: environment.getattr("integration_name")?.extract()?,
                    python: environment.getattr("python")?.extract()?,
                    direct_dependencies,
                    resolved_pkgs,
                    display_pkgs,
                    lockfile,
                    services: services.clone(),
                    suite_env: suite_env.clone(),
                    patterns: patterns.clone(),
                    runs,
                });
            }
            suites.insert(
                id.clone(),
                Suite {
                    id: id.clone(),
                    name: id,
                    patterns,
                    source_files,
                    services,
                    env: suite_env,
                    environments,
                },
            );
        }
        Ok(suites)
    })
    .map_err(|error: PyErr| RtError::message(format!("error: failed to load suitespec: {error}")))
}

fn extract_string_mapping(value: &Bound<'_, PyAny>) -> PyResult<IndexMap<String, String>> {
    let mapping: &Bound<'_, PyDict> = value.cast()?;
    let mut result = IndexMap::with_capacity(mapping.len());
    for (key, value) in mapping {
        if !value.is_none() {
            result.insert(key.extract()?, value.str()?.to_str()?.to_owned());
        }
    }
    Ok(result)
}

fn localize_service_hosts(env: &mut IndexMap<String, String>, services: &[String]) {
    for (key, value) in env.iter_mut() {
        if key.ends_with("_HOST") && services.iter().any(|service| service == value) {
            "localhost".clone_into(value);
        }
    }
}
