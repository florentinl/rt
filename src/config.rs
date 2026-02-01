use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use pyo3::{exceptions::PySystemExit, PyErr, PyResult};

pub struct RepoConfig {
    pub riotfile_path: PathBuf,
    pub riot_root: PathBuf,
    pub build_env: Arc<HashMap<String, String>>,
    pub run_env: Arc<HashMap<String, String>>,
    pub pytest_plugin_dir: PathBuf,
}

pub enum Selector {
    All,
    Pattern(String),
    Generic {
        python: Option<Vec<String>>,
        pattern: Option<String>,
    },
}

pub struct RunConfig {
    pub command_override: Option<String>,
    pub cmdargs: Vec<String>,
    pub action_label: String,
}

impl RepoConfig {
    pub fn load(
        riotfile_path: PathBuf,
        riot_root: PathBuf,
        pytest_plugin_dir: PathBuf,
    ) -> PyResult<Self> {
        let (build_env, run_env) = load_rt_toml(&riotfile_path)?;
        Ok(Self {
            riotfile_path,
            riot_root,
            build_env: Arc::new(build_env),
            run_env: Arc::new(run_env),
            pytest_plugin_dir,
        })
    }
}

fn load_rt_toml(
    riotfile_path: &Path,
) -> PyResult<(HashMap<String, String>, HashMap<String, String>)> {
    let Some(parent_dir) = riotfile_path.parent() else {
        return Ok((HashMap::new(), HashMap::new()));
    };
    let config_path = parent_dir.join("rt.toml");
    if !config_path.is_file() {
        return Ok((HashMap::new(), HashMap::new()));
    }

    let contents = fs::read_to_string(&config_path).map_err(|err| {
        eprintln!("error: failed to read {}: {err}", config_path.display());
        PyErr::new::<PySystemExit, _>(1)
    })?;

    let parsed: toml::Value = toml::from_str(&contents).map_err(|err| {
        eprintln!("error: failed to parse {}: {err}", config_path.display());
        PyErr::new::<PySystemExit, _>(1)
    })?;

    let env_table = parsed.get("env").and_then(|val| val.as_table());
    let build_env = parse_env_table(env_table.and_then(|tbl| tbl.get("build")), "env.build")?;
    let run_env = parse_env_table(env_table.and_then(|tbl| tbl.get("run")), "env.run")?;

    Ok((build_env, run_env))
}

fn parse_env_table(
    value: Option<&toml::Value>,
    section_name: &str,
) -> PyResult<HashMap<String, String>> {
    let mut env = HashMap::new();

    let Some(val) = value else {
        return Ok(env);
    };

    let table = val.as_table().ok_or_else(|| {
        eprintln!("error: {section_name} must be a table of string key/value pairs");
        PyErr::new::<PySystemExit, _>(1)
    })?;

    for (key, val) in table {
        let Some(val_str) = val.as_str() else {
            eprintln!("error: {section_name}.{key} must be a string");
            return Err(PyErr::new::<PySystemExit, _>(1));
        };
        env.insert(key.clone(), val_str.to_string());
    }

    Ok(env)
}
