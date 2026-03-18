use std::{fs, path::Path};

use indexmap::IndexSet;
use rustpython::InterpreterConfig;
use rustpython_vm::{
    PyResult, Settings, VirtualMachine, builtins::PyBaseExceptionRef, convert::TryFromObject,
    pymodule,
};
use serde_json::{Map as JsonMap, Value as JsonValue};
use serde_yaml::Value as YamlValue;

use crate::{
    config_provider::{ConfigProvider, LoadedConfig, ProviderServices, ProviderVenvNode},
    error::{RtError, RtResult},
};

pub struct RustPythonConfigProvider;

impl ConfigProvider for RustPythonConfigProvider {
    fn load(riotfile_path: &Path) -> RtResult<LoadedConfig> {
        let project_path = riotfile_path.parent().ok_or_else(|| {
            RtError::message("error: could not determine riotfile parent directory")
        })?;
        let riotfile_source = fs::read_to_string(riotfile_path)
            .map_err(|err| RtError::message(format!("error: failed to read riotfile: {err}")))?;

        let mut settings = Settings::default();
        settings
            .path_list
            .push(project_path.to_string_lossy().into_owned());

        let interpreter = InterpreterConfig::new()
            .settings(settings)
            .init_stdlib()
            .init_hook(Box::new(|vm| {
                vm.add_native_module("__rt_native".to_owned(), Box::new(rt_native::make_module));
            }))
            .interpreter();

        interpreter.enter(|vm| {
            let root = load_riotfile(vm, &riotfile_source, riotfile_path)?;
            let services = get_services(vm, project_path)?;

            Ok(LoadedConfig { root, services })
        })
    }
}

const RIOT_MODULE_SOURCE: &str = r#"
import sys
import types

riot = types.ModuleType("riot")

class Venv:
    def __init__(
        self,
        name=None,
        command=None,
        pys=None,
        pkgs=None,
        env=None,
        venvs=None,
        create=None,
        skip_dev_install=None,
    ):
        self.name = name
        self.command = command
        self.pys = pys
        self.pkgs = pkgs
        self.env = env
        self.venvs = [] if venvs is None else list(venvs)
        self.create = create
        self.skip_dev_install = skip_dev_install

riot.Venv = Venv
sys.modules["riot"] = riot

def __rt_string_list(value):
    if value is None:
        return []
    if isinstance(value, str):
        return [value]
    try:
        iterator = iter(value)
    except TypeError:
        return [str(value)]

    result = []
    for item in iterator:
        if item is None:
            continue
        result.append(str(item))
    return result

def __rt_dedup(values):
    result = []
    seen = set()
    for value in values:
        if value in seen:
            continue
        seen.add(value)
        result.append(value)
    return result

def __rt_map_of_lists(value):
    if not value:
        return {}

    result = {}
    for key, item in value.items():
        if item is None:
            continue
        result[str(key)] = __rt_dedup(__rt_string_list(item))
    return result

def __rt_venv_to_obj(value):
    return {
        "name": getattr(value, "name", None),
        "command": getattr(value, "command", None),
        "pys": __rt_string_list(getattr(value, "pys", None)),
        "pkgs": __rt_map_of_lists(getattr(value, "pkgs", None)),
        "env": __rt_map_of_lists(getattr(value, "env", None)),
        "create": getattr(value, "create", None),
        "skip_dev_install": getattr(value, "skip_dev_install", None),
        "venvs": [
            __rt_venv_to_obj(item)
            for item in (getattr(value, "venvs", None) or [])
        ],
    }
"#;

const RIOTFILE_RESULT_SOURCE: &str = r#"
import json

if "venv" not in globals():
    raise NameError("__rt_missing_venv__")

__rt_result_json = json.dumps(__rt_venv_to_obj(venv))
"#;

const RUAMEL_MODULE_SOURCE: &str = r#"
import json
import sys
import types
import __rt_native

yaml_module = types.ModuleType("ruamel.yaml")

class YAML:
    def __enter__(self):
        return self

    def __exit__(self, exc_type, exc, tb):
        return False

    def load(self, source):
        if hasattr(source, "__fspath__"):
            source = source.__fspath__()
        return json.loads(__rt_native.load_yaml(str(source)))

yaml_module.YAML = YAML

ruamel_module = types.ModuleType("ruamel")
ruamel_module.yaml = yaml_module

sys.modules["ruamel"] = ruamel_module
sys.modules["ruamel.yaml"] = yaml_module
"#;

fn load_riotfile(
    vm: &VirtualMachine,
    source: &str,
    riotfile_path: &Path,
) -> RtResult<ProviderVenvNode> {
    load_fake_riot_module(vm)?;

    let source_path = riotfile_path.to_string_lossy().into_owned();
    let script = format!("{RIOT_MODULE_SOURCE}\n{source}\n{RIOTFILE_RESULT_SOURCE}");
    let json = run_json_script(vm, &script, &source_path).map_err(|err| {
        let rendered = render_vm_exception(vm, &err);
        if rendered.contains("__rt_missing_venv__") {
            RtError::message("error: riotfile does not define a `venv` variable")
        } else {
            RtError::message(format!(
                "error: could not load riotfile.py in the python interpreter: {rendered}"
            ))
        }
    })?;

    let mut root: ProviderVenvNode = serde_json::from_str(&json)
        .map_err(|err| RtError::message(format!("error: `venv` variable is invalid: {err}")))?;
    normalize_provider_node(&mut root);

    Ok(root)
}

fn load_fake_riot_module(vm: &VirtualMachine) -> RtResult<()> {
    run_support_script(vm, RIOT_MODULE_SOURCE, "<rt_rustpython_riot>").map_err(|err| {
        RtError::message(format!(
            "error: could not load the `riot` module in the interpreter: {}",
            render_vm_exception(vm, &err)
        ))
    })
}

fn get_services(vm: &VirtualMachine, project_path: &Path) -> RtResult<Option<ProviderServices>> {
    load_fake_ruamel_module(vm)?;

    let script = format!(
        r#"{RUAMEL_MODULE_SOURCE}
import json
import sys
sys.path.insert(0, r"{}")

from tests.suitespec import SUITESPEC

__rt_result_json = json.dumps(
    {{
        key.split("::")[-1]: (
            suite.get("services", []) + ["testagent"] if suite.get("snapshot") else []
        )
        for key, suite in SUITESPEC["suites"].items()
        if suite.get("services") or suite.get("snapshot")
    }}
)
"#,
        project_path.to_string_lossy()
    );

    let Ok(json) = run_json_script(vm, &script, "<rt_rustpython_services>") else {
        return Ok(None);
    };

    serde_json::from_str(&json).map(Some).or(Ok(None))
}

fn load_fake_ruamel_module(vm: &VirtualMachine) -> RtResult<()> {
    run_support_script(vm, RUAMEL_MODULE_SOURCE, "<rt_rustpython_ruamel>").map_err(|err| {
        RtError::message(format!(
            "error: could not properly load ruamel.yaml module: {}",
            render_vm_exception(vm, &err)
        ))
    })
}

fn run_support_script(
    vm: &VirtualMachine,
    source: &str,
    source_path: &str,
) -> Result<(), PyBaseExceptionRef> {
    let scope = vm.new_scope_with_builtins();
    vm.run_code_string(scope, source, source_path.to_owned())
        .map(|_| ())
}

fn run_json_script(
    vm: &VirtualMachine,
    source: &str,
    source_path: &str,
) -> Result<String, PyBaseExceptionRef> {
    let scope = vm.new_scope_with_builtins();
    vm.run_code_string(scope.clone(), source, source_path.to_owned())?;
    let json_obj = vm.run_block_expr(scope, "__rt_result_json")?;
    String::try_from_object(vm, json_obj)
}

fn render_vm_exception(vm: &VirtualMachine, err: &PyBaseExceptionRef) -> String {
    let mut buffer = String::new();
    if vm.write_exception(&mut buffer, err).is_ok() {
        let rendered = buffer.trim().to_string();
        if !rendered.is_empty() {
            return rendered;
        }
    }
    format!("{err:?}")
}

fn normalize_provider_node(node: &mut ProviderVenvNode) {
    node.pys.retain(|value| !value.is_empty());
    node.pys
        .sort_by(|left, right| compare_python_versions(left, right));
    node.pys.dedup();

    for values in node.pkgs.values_mut() {
        *values = dedup_preserving_order(std::mem::take(values));
    }
    for values in node.env.values_mut() {
        *values = dedup_preserving_order(std::mem::take(values));
    }
    for child in &mut node.venvs {
        normalize_provider_node(child);
    }
}

fn dedup_preserving_order(values: Vec<String>) -> Vec<String> {
    let mut seen = IndexSet::new();
    values
        .into_iter()
        .filter_map(|value| seen.insert(value.clone()).then_some(value))
        .collect()
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

#[pymodule]
mod rt_native {
    use super::{Path, PyResult, VirtualMachine, YamlValue, fs, yaml_to_json_value};

    #[pyfunction]
    fn load_yaml(source: String, vm: &VirtualMachine) -> PyResult<String> {
        let yaml_text = if Path::new(&source).exists() {
            fs::read_to_string(&source)
                .map_err(|err| vm.new_value_error(format!("YAML read error ({source}): {err}")))?
        } else {
            source
        };

        let value: YamlValue = serde_yaml::from_str(&yaml_text)
            .map_err(|err| vm.new_value_error(format!("YAML parse error: {err}")))?;
        let json = yaml_to_json_value(value);

        serde_json::to_string(&json)
            .map_err(|err| vm.new_value_error(format!("YAML serialization error: {err}")))
    }
}

fn yaml_to_json_value(value: YamlValue) -> JsonValue {
    match value {
        YamlValue::Null => JsonValue::Null,
        YamlValue::Bool(value) => JsonValue::Bool(value),
        YamlValue::Number(value) => value
            .as_i64()
            .map(JsonValue::from)
            .or_else(|| value.as_u64().map(JsonValue::from))
            .or_else(|| value.as_f64().map(JsonValue::from))
            .unwrap_or(JsonValue::Null),
        YamlValue::String(value) => JsonValue::String(value),
        YamlValue::Sequence(values) => {
            JsonValue::Array(values.into_iter().map(yaml_to_json_value).collect())
        }
        YamlValue::Mapping(values) => {
            let mut map = JsonMap::new();
            for (key, value) in values {
                map.insert(yaml_key_to_string(key), yaml_to_json_value(value));
            }
            JsonValue::Object(map)
        }
        YamlValue::Tagged(tagged) => yaml_to_json_value(tagged.value),
    }
}

fn yaml_key_to_string(value: YamlValue) -> String {
    match value {
        YamlValue::String(value) => value,
        other => match yaml_to_json_value(other) {
            JsonValue::String(value) => value,
            value => value.to_string(),
        },
    }
}

#[cfg(all(test, feature = "provider-pyo3"))]
mod tests {
    use std::path::Path;

    use super::RustPythonConfigProvider;
    use crate::config_provider::{ConfigProvider, Pyo3ConfigProvider};

    #[test]
    fn rustpython_provider_matches_pyo3_on_simple_fixture() {
        let path = Path::new("tests/data/real_use_riotfile.py");
        assert_eq!(
            RustPythonConfigProvider::load(path).unwrap(),
            Pyo3ConfigProvider::load(path).unwrap()
        );
    }

    #[test]
    fn rustpython_provider_matches_pyo3_on_nested_fixture() {
        let path = Path::new("tests/data/nested_riotfile.py");
        assert_eq!(
            RustPythonConfigProvider::load(path).unwrap(),
            Pyo3ConfigProvider::load(path).unwrap()
        );
    }
}
