use std::path::Path;

use indexmap::IndexMap;
use serde::Serialize;
use serde_json::to_string_pretty;

use crate::{
    config::{RepoConfig, Selector},
    error::{RtError, RtResult},
    ui,
    venv::{ExecutionContext, RiotVenv, select_execution_contexts, venv_path},
};

#[derive(Serialize)]
struct JsonExecutionContext {
    hash: String,
    venv_path: String,
    python_path: String,
    activate_path: String,
    display_name: String,
    short_display_name: String,
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
    resolved_pkgs: IndexMap<String, String>,
    display_pkgs: IndexMap<String, String>,
    execution_contexts: Vec<JsonExecutionContext>,
}

fn python_path(venv_path: &str) -> String {
    Path::new(venv_path)
        .join("bin/python")
        .to_string_lossy()
        .into_owned()
}

fn activate_path(venv_path: &str) -> String {
    Path::new(venv_path)
        .join("bin/activate")
        .to_string_lossy()
        .into_owned()
}

fn format_entries(map: &IndexMap<String, String>, max_entries: usize) -> Option<String> {
    if map.is_empty() {
        return None;
    }

    let mut entries: Vec<_> = map.iter().collect();
    entries.sort_by_key(|(key, _)| key.as_str());

    let shown: Vec<String> = entries
        .iter()
        .take(max_entries)
        .map(|(key, value)| {
            if value.is_empty() {
                format!("{key}=latest")
            } else {
                format!("{key}={value}")
            }
        })
        .collect();

    let remaining = entries.len().saturating_sub(shown.len());
    let tail = if remaining > 0 {
        format!(" +{remaining} more")
    } else {
        String::new()
    };

    Some(format!("{}{tail}", shown.join(", ")))
}

fn unique_packages(venv: &RiotVenv) -> IndexMap<String, String> {
    let diff: IndexMap<String, String> = venv
        .pkgs
        .iter()
        .filter(|(key, value)| {
            venv.shared_pkgs
                .get(key.as_str())
                .is_none_or(|shared| shared != *value)
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    if diff.is_empty() { venv.pkgs.clone() } else { diff }
}

fn context_env_diff(ctx: &ExecutionContext, venv: &RiotVenv) -> IndexMap<String, String> {
    ctx.env
        .iter()
        .filter(|(key, value)| {
            venv.shared_env
                .get(key.as_str())
                .is_none_or(|shared| shared != *value)
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

fn build_display_names(
    venv: &RiotVenv,
    ctx: &ExecutionContext,
) -> (String, String) {
    let pkg_detail = format_entries(&unique_packages(venv), 2);
    let env_detail = format_entries(&context_env_diff(ctx, venv), 2);

    let details: Vec<&str> = [pkg_detail.as_deref(), env_detail.as_deref()]
        .into_iter()
        .flatten()
        .collect();

    let separator = " | ";

    if details.is_empty() {
        let display_name = format!("{} ({}){separator}{}", venv.name, venv.python, ctx.hash);
        return (display_name.clone(), display_name);
    }

    let display_name = format!(
        "{} ({}){separator}{}",
        venv.name,
        venv.python,
        details.join(separator)
    );

    let short_display_name = if details.len() > 1 {
        format!(
            "{} ({}){separator}{} +{} more",
            venv.name,
            venv.python,
            details[0],
            details.len() - 1
        )
    } else {
        display_name.clone()
    };

    (display_name, short_display_name)
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
) -> RtResult<()> {
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
                    .iter()
                    .map(|ctx| {
                        let ctx_venv_path =
                            venv_path(&repo.riot_root, &ctx.hash).display().to_string();
                        let (display_name, short_display_name) =
                            build_display_names(&venv, ctx);
                        JsonExecutionContext {
                            hash: ctx.hash.clone(),
                            python_path: python_path(&ctx_venv_path),
                            activate_path: activate_path(&ctx_venv_path),
                            display_name,
                            short_display_name,
                            venv_path: ctx_venv_path,
                            command: ctx.command.clone(),
                            pytest_target: ctx.pytest_target.clone(),
                            env: ctx.env.clone(),
                            create: ctx.create,
                            skip_dev_install: ctx.skip_dev_install,
                        }
                    })
                    .collect();

                JsonVenv {
                    hash: venv.hash.clone(),
                    venv_path: venv_path(&repo.riot_root, &venv.hash).display().to_string(),
                    name: venv.name,
                    python: venv.python,
                    pkgs: venv.pkgs,
                    resolved_pkgs: venv.resolved_pkgs,
                    display_pkgs: venv.display_pkgs,
                    services: venv.services,
                    execution_contexts,
                }
            })
            .collect();

        let output = to_string_pretty(&json_venvs).map_err(|err| {
            RtError::message(format!("error: failed to serialize venvs as JSON: {err}"))
        })?;
        println!("{output}");
        return Ok(());
    }

    ui::print_venv_hierarchy(&venvs, |line| println!("{line}"));

    Ok(())
}
