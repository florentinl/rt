use std::{
    env,
    ffi::OsString,
    process::{Command, Stdio},
};

use indexmap::IndexMap;
use pyo3::{exceptions::PySystemExit, PyErr, PyResult};

use crate::{
    commands::build::build_selected_contexts,
    config::{RepoConfig, Selector},
    ui::{self},
    venv::{select_execution_contexts, venv_python_path, ExecutionContext, RiotVenv},
};

/// Build the requested environment and spawn an interactive shell with it active.
///
/// # Errors
///
/// Returns an error if the target cannot be resolved, built, or launched.
pub fn run(
    venvs: IndexMap<String, RiotVenv>,
    repo: &RepoConfig,
    hash: &str,
    force_reinstall: bool,
) -> PyResult<()> {
    let target = resolve_target(venvs, hash)?;

    build_selected_contexts(repo, std::slice::from_ref(&target), force_reinstall)?;
    let ctx = &target.execution_contexts[0];
    ui::step(format!("Spawning shell for execution context {}", ctx.hash));

    launch_shell(repo, ctx)?;

    Ok(())
}

/// Resolve a selector to a single venv, narrowing to an execution context when needed.
///
/// # Errors
///
/// Returns an error if selection is ambiguous or selection fails.
pub fn resolve_target(venvs: IndexMap<String, RiotVenv>, hash: &str) -> PyResult<RiotVenv> {
    let mut venvs = select_execution_contexts(venvs, Selector::Pattern(hash.to_string()))?;
    if venvs.len() != 1 {
        eprintln!("Found multiple corresponding virtual environments, aborting...");
        return Err(PyErr::new::<PySystemExit, _>(1));
    }
    let Some(mut venv) = venvs.pop() else {
        eprintln!("Found multiple corresponding virtual environments, aborting...");
        return Err(PyErr::new::<PySystemExit, _>(1));
    };

    venv.execution_contexts.retain(|exc| exc.hash == hash);

    let n_ctx = venv.execution_contexts.len();
    if n_ctx >= 2 {
        eprintln!("Found multiple corresponding virtual environments, aborting...");
        return Err(PyErr::new::<PySystemExit, _>(1));
    }

    if n_ctx == 1 {
        return Ok(venv);
    }

    venv.execution_contexts.push(make_venv_shell_context(&venv));
    Ok(venv)
}

#[must_use] 
pub fn make_venv_shell_context(venv: &RiotVenv) -> ExecutionContext {
    ExecutionContext {
        command: None,
        pytest_target: None,
        env: IndexMap::new(),
        create: false,
        skip_dev_install: false,
        hash: venv.hash.clone(),
    }
}

fn launch_shell(repo: &RepoConfig, exc_ctx: &ExecutionContext) -> PyResult<()> {
    let python_path = venv_python_path(&repo.riot_root, &exc_ctx.hash);

    let shell = preferred_shell();
    ui::detail(format!(
        "Starting {} with virtual environment {} active",
        shell.to_string_lossy(),
        exc_ctx.hash
    ));

    let mut command = Command::new("uv");
    command
        .arg("run")
        .arg("--no-config")
        .arg("--color=always")
        .arg("--no-project")
        .arg("--python")
        .arg(&python_path)
        .arg("--")
        .arg(&shell)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .env("UV_PYTHON_PREFERENCE", "only-managed")
        .env("FORCE_COLOR", "1");

    for (key, value) in &exc_ctx.env {
        command.env(key, value);
    }

    command.envs(repo.run_env.iter());

    let status = command.status().map_err(|err| {
        eprintln!("error: failed to spawn shell for {}: {err}", exc_ctx.hash);
        PyErr::new::<PySystemExit, _>(1)
    })?;

    if status.success() {
        Ok(())
    } else {
        Err(PyErr::new::<PySystemExit, _>(status.code().unwrap_or(1)))
    }
}

fn preferred_shell() -> OsString {
    env::var_os("SHELL")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            if cfg!(windows) {
                OsString::from("cmd.exe")
            } else {
                OsString::from("sh")
            }
        })
}
