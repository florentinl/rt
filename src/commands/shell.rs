use std::{
    env,
    ffi::OsString,
    process::{Command, Stdio},
};

use indexmap::IndexMap;

use crate::{
    commands::build::build_selected_environments,
    config::{RepoConfig, Selector},
    error::{RtError, RtResult},
    ui,
    venv::{Suite, TestEnvironment, profile_path, select_test_environments},
};

pub fn run(
    suites: &IndexMap<String, Suite>,
    repo: &RepoConfig,
    selector: &str,
    force_reinstall: bool,
) -> RtResult<()> {
    let target = resolve_target(suites, selector)?;
    build_selected_environments(repo, std::slice::from_ref(&target), force_reinstall, false)?;
    let run = target
        .runs
        .first()
        .ok_or_else(|| RtError::message("error: test environment has no run"))?;
    ui::step(format!("Spawning shell for test run {}", run.id));
    let profile = profile_path(&repo.cache_root, &target.hash, run.index);
    let physical = crate::venv::environment_path(&repo.cache_root, &target.hash);
    let shell = preferred_shell();
    let mut command = Command::new(&shell);
    command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .current_dir(&repo.project_root)
        .env("VIRTUAL_ENV", &physical)
        .env(
            "PATH",
            format!(
                "{}:{}:{}",
                profile.join("bin").display(),
                physical.join("bin").display(),
                env::var("PATH").unwrap_or_default()
            ),
        )
        .env(
            "RT_CONTEXT_PATH",
            profile.join(crate::constants::RUNTIME_CONFIG),
        )
        .envs(&target.suite_env)
        .envs(&run.env);
    let status = command.status().map_err(|error| {
        RtError::message(format!(
            "error: failed to spawn shell for {}: {error}",
            run.id
        ))
    })?;
    if status.success() {
        Ok(())
    } else {
        Err(RtError::silent_from_status(status))
    }
}

pub fn resolve_target(
    suites: &IndexMap<String, Suite>,
    selector: &str,
) -> RtResult<TestEnvironment> {
    let mut selected = select_test_environments(suites, Selector::Pattern(selector.to_owned()))?;
    if selected.len() != 1 {
        return Err(RtError::message(format!(
            "Expected one test environment, found {}",
            selected.len()
        )));
    }
    let mut environment = selected.pop().expect("one selected environment");
    if environment.runs.len() > 1 {
        if let Some(run) = environment
            .runs
            .iter()
            .find(|run| run.id == selector)
            .cloned()
        {
            environment.runs = vec![run];
        } else {
            environment.runs.truncate(1);
        }
    }
    Ok(environment)
}

fn preferred_shell() -> OsString {
    env::var_os("SHELL")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| OsString::from("sh"))
}
