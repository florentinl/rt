use indexmap::IndexMap;
use serde::Serialize;

use crate::{
    config::{RepoConfig, Selector},
    error::{RtError, RtResult},
    ui,
    venv::{
        Suite, TestEnvironment, TestRun, environment_path, profile_fingerprint, profile_path,
        pytest_args, pytest_targets, select_test_environments,
    },
};

#[derive(Serialize)]
struct JsonIndex {
    schema_version: u8,
    suites: Vec<JsonSuite>,
}

#[derive(Serialize)]
struct JsonSuite {
    id: String,
    name: String,
    patterns: Vec<String>,
    source_files: Vec<String>,
    services: Vec<String>,
    suite_env: IndexMap<String, String>,
    environments: Vec<JsonEnvironment>,
}

#[derive(Serialize)]
struct JsonEnvironment {
    hash: String,
    name: String,
    integration_name: String,
    python: String,
    direct_dependencies: Vec<String>,
    resolved_pkgs: IndexMap<String, String>,
    display_pkgs: IndexMap<String, String>,
    lockfile: String,
    venv_path: String,
    python_path: String,
    activate_path: String,
    runs: Vec<JsonRun>,
}

#[derive(Serialize)]
struct JsonRun {
    id: String,
    index: usize,
    command: String,
    env: IndexMap<String, String>,
    pytest_targets: Vec<String>,
    pytest_args: Vec<String>,
    profile_fingerprint: String,
    profile_path: String,
    venv_path: String,
    python_path: String,
    activate_path: String,
    display_name: String,
    short_display_name: String,
}

pub fn run(
    suites: IndexMap<String, Suite>,
    repo: &RepoConfig,
    selector: Selector,
    hash_only: bool,
    json: bool,
) -> RtResult<()> {
    let selected = select_test_environments(&suites, selector)?;
    if hash_only {
        let mut hashes: Vec<_> = selected
            .iter()
            .map(|environment| &environment.hash)
            .collect();
        hashes.sort();
        hashes.dedup();
        for hash in hashes {
            println!("{hash}");
        }
        return Ok(());
    }
    if !json {
        ui::print_environment_hierarchy(&selected, |line| println!("{line}"));
        return Ok(());
    }
    let selected_hashes = selected
        .iter()
        .map(|environment| environment.hash.as_str())
        .collect::<std::collections::HashSet<_>>();
    let mut json_suites = Vec::new();
    for suite in suites.into_values() {
        let environments: Vec<_> = suite
            .environments
            .iter()
            .filter(|environment| selected_hashes.contains(environment.hash.as_str()))
            .map(|environment| json_environment(repo, environment))
            .collect();
        if environments.is_empty() {
            continue;
        }
        json_suites.push(JsonSuite {
            id: suite.id,
            name: suite.name,
            patterns: suite.patterns,
            source_files: suite.source_files,
            services: suite.services,
            suite_env: suite.env,
            environments,
        });
    }
    let output = serde_json::to_string_pretty(&JsonIndex {
        schema_version: 1,
        suites: json_suites,
    })
    .map_err(|error| {
        RtError::message(format!(
            "error: failed to serialize suitespec index: {error}"
        ))
    })?;
    println!("{output}");
    Ok(())
}

fn json_environment(repo: &RepoConfig, environment: &TestEnvironment) -> JsonEnvironment {
    let physical = environment_path(&repo.cache_root, &environment.hash);
    let python = physical.join(if cfg!(windows) {
        "Scripts/python.exe"
    } else {
        "bin/python"
    });
    let activate = physical.join(if cfg!(windows) {
        "Scripts/activate"
    } else {
        "bin/activate"
    });
    JsonEnvironment {
        hash: environment.hash.clone(),
        name: environment.name.clone(),
        integration_name: environment.integration_name.clone(),
        python: environment.python.clone(),
        direct_dependencies: environment.direct_dependencies.clone(),
        resolved_pkgs: environment.resolved_pkgs.clone(),
        display_pkgs: environment.display_pkgs.clone(),
        lockfile: environment.lockfile.clone(),
        venv_path: physical.display().to_string(),
        python_path: python.display().to_string(),
        activate_path: activate.display().to_string(),
        runs: environment
            .runs
            .iter()
            .map(|run| json_run(repo, environment, run))
            .collect(),
    }
}

fn json_run(repo: &RepoConfig, environment: &TestEnvironment, run: &TestRun) -> JsonRun {
    let profile = profile_path(&repo.cache_root, &environment.hash, run.index);
    JsonRun {
        id: run.id.clone(),
        index: run.index,
        command: run.command.clone(),
        env: run.env.clone(),
        pytest_targets: pytest_targets(&run.command),
        pytest_args: pytest_args(&run.command),
        profile_fingerprint: profile_fingerprint(environment, run, false),
        profile_path: profile
            .join(crate::constants::RUNTIME_CONFIG)
            .display()
            .to_string(),
        venv_path: profile.display().to_string(),
        python_path: profile.join("bin/python").display().to_string(),
        activate_path: profile.join("bin/activate").display().to_string(),
        display_name: format!(
            "{} / {} (Python {}) / run {}",
            environment.suite,
            environment.name,
            environment.python,
            run.index + 1
        ),
        short_display_name: format!(
            "{} ({}) #{}",
            environment.name,
            environment.python,
            run.index + 1
        ),
    }
}
