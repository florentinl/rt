use std::{fs, path::Path};

use crate::{
    constants::{RT_VERSION, RUNTIME_CONFIG},
    error::{RtError, RtResult},
    venv::{venv_path, venv_python_path},
};
use serde::Deserialize;

#[derive(Deserialize)]
struct RuntimeConfig {
    rt_version: String,
    #[serde(rename = "project_root")]
    _project_root: String,
    #[serde(rename = "no_editable")]
    _no_editable: bool,
    #[serde(rename = "environment")]
    _environment: std::collections::HashMap<String, String>,
    #[serde(rename = "services")]
    _services: Vec<String>,
    command: serde_json::Value,
}

/// Report whether an execution-context environment was completely built by
/// this version of rt.
///
/// This is an internal command used by integrations. Keeping the readiness
/// policy here prevents callers from depending on rt's marker-file layout.
pub fn run(riot_root: &Path, hash: &str) -> RtResult<()> {
    if hash.is_empty()
        || !hash.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '@' | '_' | '-')
        })
    {
        return Err(RtError::with_code(2, "error: invalid venv hash"));
    }

    println!("{}", is_ready(riot_root, hash));
    Ok(())
}

fn is_ready(riot_root: &Path, hash: &str) -> bool {
    let environment_path = venv_path(riot_root, hash);
    let manifest_is_current = fs::read_to_string(environment_path.join(RUNTIME_CONFIG))
        .ok()
        .and_then(|contents| serde_json::from_str::<RuntimeConfig>(&contents).ok())
        .is_some_and(|manifest| {
            manifest.rt_version == RT_VERSION
                && (manifest.command.is_null() || manifest.command.is_string())
        });

    manifest_is_current && Path::new(&venv_python_path(riot_root, hash)).is_file()
}

#[cfg(test)]
mod tests {
    use super::is_ready;
    use crate::{
        constants::{RT_VERSION, RUNTIME_CONFIG},
        venv::venv_path,
    };
    use std::fs;

    #[test]
    fn readiness_requires_python_and_a_current_runtime_manifest() {
        let root = tempfile::tempdir().unwrap();
        let hash = "abc1234@def5678";
        let environment = venv_path(root.path(), hash);
        fs::create_dir_all(environment.join("bin")).unwrap();

        assert!(!is_ready(root.path(), hash));

        fs::write(environment.join("bin/python"), "").unwrap();
        assert!(!is_ready(root.path(), hash));

        fs::write(environment.join(RUNTIME_CONFIG), "not json\n").unwrap();
        assert!(!is_ready(root.path(), hash));

        fs::write(
            environment.join(RUNTIME_CONFIG),
            "{\"rt_version\":\"an-older-version\"}\n",
        )
        .unwrap();
        assert!(!is_ready(root.path(), hash));

        fs::write(
            environment.join(RUNTIME_CONFIG),
            format!(
                concat!(
                    "{{",
                    "\"rt_version\":\"{}\",",
                    "\"project_root\":\"/project\",",
                    "\"no_editable\":false,",
                    "\"environment\":{{}},",
                    "\"services\":[],",
                    "\"command\":null",
                    "}}\n"
                ),
                RT_VERSION
            ),
        )
        .unwrap();
        assert!(is_ready(root.path(), hash));
    }
}
