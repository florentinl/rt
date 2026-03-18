use pyo3::IntoPyObjectExt;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyDict, PyList, PyModule};
use serde_yaml::Value as YamlValue;
use std::fs;

fn yaml_to_py<'a>(py: Python<'a>, v: &YamlValue) -> PyResult<Bound<'a, PyAny>> {
    match v {
        YamlValue::Null => Ok(py.None().into_bound(py)),
        YamlValue::Bool(b) => b.into_bound_py_any(py),
        YamlValue::Number(n) => match (n.as_i64(), n.as_u64(), n.as_f64()) {
            (Some(i), _, _) => i.into_bound_py_any(py),
            (None, Some(u), _) => u.into_bound_py_any(py),
            (None, None, Some(f)) => f.into_bound_py_any(py),
            (None, None, None) => Err(PyErr::new::<PyValueError, _>("Unsupported YAML number")),
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
            for (key, value) in map {
                let py_key = yaml_to_py(py, key)?;
                let py_value = yaml_to_py(py, value)?;
                dict.set_item(py_key, py_value)?;
            }
            Ok(dict.into_any())
        }
        YamlValue::Tagged(tagged) => yaml_to_py(py, &tagged.value),
    }
}

#[pyclass(name = "YAML")]
struct Yaml;

#[pymethods]
impl Yaml {
    #[new]
    const fn new() -> Self {
        Self
    }

    const fn __enter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    const fn __exit__(
        &self,
        _exc_type: &Bound<'_, PyAny>,
        _exc: &Bound<'_, PyAny>,
        _tb: &Bound<'_, PyAny>,
    ) -> bool {
        let _ = self;
        false
    }

    fn load(&self, py: Python<'_>, source: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let _ = self;
        let yaml_text = if let Ok(path) = source
            .call_method0("__fspath__")
            .and_then(|value| value.extract::<String>())
        {
            fs::read_to_string(&path).map_err(|err| {
                PyErr::new::<PyValueError, _>(format!("YAML read error ({path}): {err}"))
            })?
        } else if let Ok(text) = source.extract::<String>() {
            text
        } else {
            return Err(PyErr::new::<PyValueError, _>(
                "YAML.load expects pathlib.Path or YAML string",
            ));
        };

        let parsed: YamlValue = serde_yaml::from_str(&yaml_text)
            .map_err(|err| PyErr::new::<PyValueError, _>(format!("YAML parse error: {err}")))?;
        yaml_to_py(py, &parsed).map(Bound::unbind)
    }
}

pub fn get_fake_ruamel_yaml(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    let yaml_module = PyModule::new(py, "ruamel.yaml")?;
    yaml_module.add_class::<Yaml>()?;
    Ok(yaml_module)
}
