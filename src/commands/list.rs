use indexmap::IndexMap;
use pyo3::{exceptions::PySystemExit, prelude::*};
use serde::Serialize;
use serde_json::to_string_pretty;

use crate::{
    config::{RepoConfig, Selector},
    ui,
    venv::{select_execution_contexts, venv_path, RiotVenv},
};

#[derive(Serialize)]
struct JsonExecutionContext {
    hash: String,
    venv_path: String,
    command: Option<String>,
    pytest_target: Option<String>,
    env: IndexMap<String, String>,
    create: bool,
    skip_dev_install: bool,
}

#[derive(Serialize)]
struct JsonVenv {
    hash: String,
    venv_path: String,
    name: String,
    python: String,
    services: Vec<String>,
    pkgs: IndexMap<String, String>,
    shared_pkgs: IndexMap<String, String>,
    shared_env: IndexMap<String, String>,
    execution_contexts: Vec<JsonExecutionContext>,
}

/// List selected virtual environments in tree, hash-only, or JSON format.
///
/// # Errors
///
/// Returns an error if context selection or JSON serialization fails.
pub fn run(
    venvs: IndexMap<String, RiotVenv>,
    repo: &RepoConfig,
    selector: Selector,
    hash_only: bool,
    json: bool,
) -> PyResult<()> {
    let mut venvs = select_execution_contexts(venvs, selector)?;

    venvs.retain(|v| !v.execution_contexts.is_empty());

    if venvs.is_empty() {
        return Ok(());
    }

    if hash_only {
        let mut keys: Vec<&str> = venvs.iter().map(|v| v.hash.as_str()).collect();
        keys.sort_unstable();
        for hash in keys {
            println!("{hash}");
        }
        return Ok(());
    }

    if json {
        for venv in &mut venvs {
            venv.execution_contexts
                .sort_by(|left, right| left.hash.cmp(&right.hash));
        }
        venvs.sort_by(|left, right| left.hash.cmp(&right.hash));

        let json_venvs: Vec<JsonVenv> = venvs
            .into_iter()
            .map(|venv| {
                let execution_contexts = venv
                    .execution_contexts
                    .into_iter()
                    .map(|ctx| JsonExecutionContext {
                        hash: ctx.hash.clone(),
                        venv_path: venv_path(&repo.riot_root, &ctx.hash).display().to_string(),
                        command: ctx.command,
                        pytest_target: ctx.pytest_target,
                        env: ctx.env,
                        create: ctx.create,
                        skip_dev_install: ctx.skip_dev_install,
                    })
                    .collect();

                JsonVenv {
                    hash: venv.hash.clone(),
                    venv_path: venv_path(&repo.riot_root, &venv.hash).display().to_string(),
                    name: venv.name,
                    python: venv.python,
                    pkgs: venv.pkgs,
                    services: venv.services,
                    shared_pkgs: venv.shared_pkgs,
                    shared_env: venv.shared_env,
                    execution_contexts,
                }
            })
            .collect();

        let output = to_string_pretty(&json_venvs).map_err(|err| {
            eprintln!("error: failed to serialize venvs as JSON: {err}");
            PyErr::new::<PySystemExit, _>(1)
        })?;
        println!("{output}");
        return Ok(());
    }

    ui::print_venv_hierarchy(&venvs, |line| println!("{line}"));

    Ok(())
}
