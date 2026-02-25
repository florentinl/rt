use pyo3::IntoPyObjectExt;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyDict, PyList, PyModule};
use serde_yaml::Value as YamlValue;
use std::fs;

/// Convert a `serde_yaml::Value` into a native Python object.
fn yaml_to_py<'a>(py: Python<'a>, v: &YamlValue) -> PyResult<Bound<'a, PyAny>> {
    match v {
        YamlValue::Null => Ok(py.None().into_bound(py)),
        YamlValue::Bool(b) => b.into_bound_py_any(py),
        YamlValue::Number(n) => match (n.as_i64(), n.as_u64(), n.as_f64()) {
            (Some(i), _, _) => i.into_bound_py_any(py),
            (None, Some(u), _) => u.into_bound_py_any(py),
            (None, None, Some(f)) => f.into_bound_py_any(py),
            (None, None, None) => {
                // Extremely rare, but keep it safe.
                Err(PyErr::new::<PyValueError, _>("Unsupported YAML number"))
            }
        },
        YamlValue::String(s) => s.as_str().into_bound_py_any(py),
        YamlValue::Sequence(seq) => {
            let list = PyList::empty(py);
            for item in seq {
                list.append(yaml_to_py(py, item)?)?;
            }
            Ok(list.into_any())
        }
        YamlValue::Mapping(map) => {
            let dict = PyDict::new(py);
            for (k, val) in map {
                // YAML allows non-string keys; Python dict supports that.
                let py_key = yaml_to_py(py, k)?;
                let py_val = yaml_to_py(py, val)?;
                dict.set_item(py_key, py_val)?;
            }
            Ok(dict.into_any())
        }
        YamlValue::Tagged(tagged) => {
            // Minimal behavior: ignore tag and return underlying value.
            // (Later you could preserve tags or create tagged wrapper objects.)
            yaml_to_py(py, &tagged.value)
        }
    }
}

#[pyclass(name = "YAML")]
struct Yaml {}

#[pymethods]
impl Yaml {
    #[new]
    const fn new() -> Self {
        Self {}
    }

    /// Context-manager enter: `with YAML() as yaml: ...`
    const fn __enter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// Context-manager exit: no special cleanup for now.
    const fn __exit__(
        &self,
        _exc_type: &Bound<'_, PyAny>,
        _exc: &Bound<'_, PyAny>,
        _tb: &Bound<'_, PyAny>,
    ) -> bool {
        let _ = self;
        false // returning false means "do not suppress exceptions"
    }

    /// Parse YAML from a file path (pathlib.Path) or from a raw YAML string.
    fn load(&self, py: Python<'_>, source: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let _ = self;
        let yaml_text = if let Ok(path) = source
            .call_method0("__fspath__")
            .and_then(|p| p.extract::<String>())
        {
            fs::read_to_string(&path)
                .map_err(|e| PyErr::new::<PyValueError, _>(format!("YAML read error ({path}): {e}")))?
        } else if let Ok(text) = source.extract::<String>() {
            text
        } else {
            return Err(PyErr::new::<PyValueError, _>(
                "YAML.load expects pathlib.Path or YAML string",
            ));
        };

        let v: YamlValue = serde_yaml::from_str(&yaml_text)
            .map_err(|e| PyErr::new::<PyValueError, _>(format!("YAML parse error: {e}")))?;
        yaml_to_py(py, &v).map(Bound::unbind)
    }
}

pub fn get_fake_ruamel_yaml(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    let yaml_module = PyModule::new(py, "ruamel.yaml")?;
    yaml_module.add_class::<Yaml>()?;
    Ok(yaml_module)
}
