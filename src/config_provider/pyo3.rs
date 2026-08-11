use std::{ffi::CString, fs, path::Path};

use indexmap::{IndexMap, IndexSet};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyAnyMethods, PyDict, PyIterator, PyModule, PyString, PyStringMethods};

use crate::{
    config_provider::{ConfigProvider, LoadedConfig, ProviderSuites, ProviderVenvNode},
    error::{RtError, RtResult},
};

// ---------------------------------------------------------------------------
// FromPyObject newtypes — let PyO3's .extract() do the dispatch
// ---------------------------------------------------------------------------

/// A value that is either a single stringified Python object, a list of them,
/// or None (→ empty vec). Covers the `pys`, `pkgs` value, and `env` value
/// slots in the riot `Venv` constructor.
struct StringOrList(Vec<String>);

impl<'a, 'py> FromPyObject<'a, 'py> for StringOrList {
    type Error = PyErr;

    fn extract(obj: Borrowed<'a, 'py, PyAny>) -> PyResult<Self> {
        if let Ok(s) = obj.cast::<PyString>() {
            return Ok(Self(vec![s.to_str()?.to_owned()]));
        }
        if let Ok(iter) = PyIterator::from_object(obj.as_any()) {
            let values: PyResult<Vec<String>> = iter
                .filter_map(|item| {
                    let item = item.ok()?;
                    if item.is_none() {
                        return None;
                    }
                    Some(item.str().and_then(|s| Ok(s.to_str()?.to_owned())))
                })
                .collect();
            return Ok(Self(values?));
        }
        // Fallback: stringify the whole object (handles bare floats like 3.8).
        Ok(Self(vec![obj.str()?.to_str()?.to_owned()]))
    }
}

/// A Python dict whose values may be strings, lists, or None, normalized to
/// `IndexMap<String, Vec<String>>`. None values are skipped, duplicates within
/// each value list are removed while preserving insertion order.
struct DictOfStringLists(IndexMap<String, Vec<String>>);

impl<'a, 'py> FromPyObject<'a, 'py> for DictOfStringLists {
    type Error = PyErr;

    fn extract(obj: Borrowed<'a, 'py, PyAny>) -> PyResult<Self> {
        let dict = obj.cast::<PyDict>()?;
        let mut map = IndexMap::with_capacity(dict.len());
        for (key, value) in dict.to_owned() {
            if value.is_none() {
                continue;
            }
            let name: String = key.extract()?;
            let StringOrList(values) = value.extract()?;
            map.insert(
                name,
                values
                    .into_iter()
                    .collect::<IndexSet<_>>()
                    .into_iter()
                    .collect(),
            );
        }
        Ok(Self(map))
    }
}

// ---------------------------------------------------------------------------
// PyVenv — the fake `riot.Venv` class exposed to the embedded interpreter
// ---------------------------------------------------------------------------

pub struct Pyo3ConfigProvider;

impl ConfigProvider for Pyo3ConfigProvider {
    fn load(riotfile_path: &Path) -> RtResult<LoadedConfig> {
        let project_path = riotfile_path.parent().ok_or_else(|| {
            RtError::message("error: could not determine riotfile parent directory")
        })?;

        Python::attach(|py| {
            py.import("gc")?.call_method0("disable")?;

            let root = load_riotfile(py, riotfile_path)?;
            let suites = get_suites(py, project_path)?;

            Ok(LoadedConfig { root, suites })
        })
    }
}

#[derive(Clone)]
#[pyclass(from_py_object, name = "Venv", module = "riot")]
pub struct PyVenv {
    name: Option<String>,
    command: Option<String>,
    pys: Vec<String>,
    pkgs: IndexMap<String, Vec<String>>,
    env: IndexMap<String, Vec<String>>,
    create: Option<bool>,
    skip_dev_install: Option<bool>,
    venvs: Vec<Self>,
}

#[pymethods]
impl PyVenv {
    #[new]
    #[allow(clippy::too_many_arguments)]
    #[pyo3(
        signature = (name=None, command=None, pys=None, pkgs=None, env=None, venvs=None, create=None, skip_dev_install=None)
    )]
    fn new(
        name: Option<String>,
        command: Option<String>,
        pys: Option<StringOrList>,
        pkgs: Option<DictOfStringLists>,
        env: Option<DictOfStringLists>,
        venvs: Option<Vec<Self>>,
        create: Option<bool>,
        skip_dev_install: Option<bool>,
    ) -> Self {
        Self {
            name,
            command,
            pys: normalize_pys(pys.map_or_else(Vec::new, |s| s.0)),
            pkgs: pkgs.map_or_else(IndexMap::new, |d| d.0),
            env: env.map_or_else(IndexMap::new, |d| d.0),
            create,
            skip_dev_install,
            venvs: venvs.unwrap_or_default(),
        }
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

// ---------------------------------------------------------------------------
// Riotfile / module loading
// ---------------------------------------------------------------------------

fn load_riotfile(py: Python<'_>, path: &Path) -> RtResult<ProviderVenvNode> {
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

    let venv: PyVenv = module.getattr("venv")?.extract().map_err(PyErr::from)?;
    Ok(venv.into())
}

fn get_suites(py: Python<'_>, project_path: &Path) -> RtResult<Option<ProviderSuites>> {
    let python_code = format!(
        "import sys; sys.path.insert(0, r\"{}\"); from tests.suitespec import get_suites; SUITES = get_suites()",
        project_path.to_string_lossy()
    );

    let Some(code_cstr) = CString::new(python_code).ok() else {
        return Ok(None);
    };

    Ok(extract_suitespec_suites(py, &code_cstr).ok())
}

fn extract_suitespec_suites(py: Python<'_>, code: &CString) -> PyResult<ProviderSuites> {
    let module = PyModule::from_code(py, code.as_c_str(), c"get_services", c"_get_service")?;
    let suites_obj = module.getattr("SUITES")?;
    let suites: &Bound<'_, PyDict> = suites_obj.cast()?;

    let mut result = ProviderSuites::new();

    for (key, value) in suites {
        let suite_name: String = key.extract()?;
        let dict: &Bound<'_, PyDict> = value.cast()?;
        let services: Option<Vec<String>> =
            dict.get_item("services")?.and_then(|v| v.extract().ok());
        let snapshot: Option<bool> = dict.get_item("snapshot")?.and_then(|v| v.extract().ok());
        let pattern: String = dict
            .get_item("pattern")?
            .map_or_else(|| Ok(suite_name.clone()), |v| v.extract())?;

        let mut svc_list = IndexSet::new();
        for service in services.unwrap_or_default() {
            svc_list.insert(service);
        }
        if snapshot.unwrap_or(false) {
            svc_list.insert("testagent".to_owned());
        }

        let env = dict
            .get_item("env")?
            .as_ref()
            .map(extract_suite_env)
            .transpose()?
            .unwrap_or_default();

        let services = svc_list.into_iter().collect::<Vec<_>>();
        if services.is_empty() && env.is_empty() {
            continue;
        }

        let entry = result.entry(pattern).or_default();
        entry.services.extend(services);
        entry.env.extend(env);
    }

    Ok(result)
}

/// Stringify a suitespec `env` mapping.
///
/// Suitespec values come from YAML so they may be booleans or integers
/// (e.g. `snapshot`-style flags or `PORT: 9126`). `scripts/run-tests` coerces
/// them with `str(value)`, so do exactly the same here.
fn extract_suite_env(value: &Bound<'_, PyAny>) -> PyResult<IndexMap<String, String>> {
    let dict: &Bound<'_, PyDict> = value.cast()?;
    let mut env = IndexMap::with_capacity(dict.len());
    for (key, value) in dict {
        if value.is_none() {
            continue;
        }
        let key: String = key.extract()?;
        env.insert(key, value.str()?.to_str()?.to_owned());
    }
    Ok(env)
}

// ---------------------------------------------------------------------------
// Pure-Rust helpers (version normalization, dedup)
// ---------------------------------------------------------------------------

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
            left.cmp(&right)
        }
        _ => lhs.cmp(rhs),
    }
}

fn parse_version_components(version: &str) -> Option<Vec<u32>> {
    if version.is_empty() {
        return Some(vec![]);
    }
    version.split('.').map(|p| p.parse::<u32>().ok()).collect()
}
