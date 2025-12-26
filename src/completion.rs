use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::OnceLock,
};

use clap::builder::StyledStr;
use clap_complete::{engine::ValueCompleter, CompletionCandidate};
use pyo3::Python;

use crate::{
    config::Selector,
    locate_riotfile,
    ui::{format_envs, format_pkgs},
    venv::{compare_python_versions, select_execution_contexts, RiotVenv},
};

struct CompletionData {
    riotfile: PathBuf,
    venvs: Vec<RiotVenv>,
}

static COMPLETION_DATA: OnceLock<CompletionData> = OnceLock::new();

fn completion_data() -> Option<&'static CompletionData> {
    COMPLETION_DATA.get()
}

fn select_contexts(pattern: &str, data: &CompletionData) -> Vec<RiotVenv> {
    Python::attach(|py| {
        select_execution_contexts(py, &data.riotfile, Selector::Pattern(pattern.to_string()))
    })
    .unwrap_or_default()
}

fn completion_requested() -> bool {
    std::env::var_os("COMPLETE").is_some_and(|value| !value.is_empty() && value != "0")
}

pub fn prepare(py: Python<'_>) {
    if completion_data().is_some() || !completion_requested() {
        return;
    }
    let Some(riotfile) = locate_riotfile(None) else {
        return;
    };

    let venvs = select_execution_contexts(py, &riotfile, Selector::All).unwrap_or_default();

    let _ = COMPLETION_DATA.set(CompletionData { riotfile, venvs });
}

pub struct PythonCompleter;

impl ValueCompleter for PythonCompleter {
    fn complete(&self, current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
        let Some(hint) = current.to_str() else {
            return vec![];
        };

        let Some(data) = completion_data() else {
            return vec![];
        };

        let mut python_version: HashSet<&String> = HashSet::new();

        for venv in &data.venvs {
            if venv.python.starts_with(hint) {
                python_version.insert(&venv.python);
            }
        }

        let mut python_version: Vec<_> = python_version.into_iter().collect();
        python_version.sort_by(|a, b| compare_python_versions(a, b));

        python_version
            .into_iter()
            .map(|value| CompletionCandidate::new(value.as_str()))
            .collect()
    }
}

pub struct NameCompleter;

impl ValueCompleter for NameCompleter {
    fn complete(&self, current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
        let Some(current) = current.to_str() else {
            return vec![];
        };

        let Some(data) = completion_data() else {
            return vec![];
        };

        let selected_venvs = select_contexts(current, data);

        let mut venv_names = HashSet::new();

        for selected in selected_venvs {
            venv_names.insert(selected.name);
        }

        let mut venv_names: Vec<_> = venv_names.iter().collect();
        venv_names.sort();

        venv_names.iter().map(CompletionCandidate::new).collect()
    }
}

fn complete_selector(current: &std::ffi::OsStr, with_names: bool) -> Vec<CompletionCandidate> {
    let Some(hint) = current.to_str() else {
        return vec![];
    };

    let Some(data) = completion_data() else {
        return vec![];
    };

    let selected = select_contexts(hint, data);

    let mut candidates: HashMap<String, CompletionCandidate> = HashMap::new();

    for selected in &selected {
        if with_names {
            candidates.insert(
                selected.name.clone(),
                CompletionCandidate::new(&selected.name),
            );
        }

        let pkgs_detail = if selected.pkgs.is_empty() {
            String::new()
        } else {
            format!(" {}", format_pkgs(&selected.pkgs, &selected.shared_pkgs),)
        };

        let short_hash = selected.hash.clone();
        let short_hash_candidate =
            CompletionCandidate::new(short_hash.as_str()).help(Some(StyledStr::from(format!(
                "{} ({}){}",
                selected.name, selected.python, pkgs_detail
            ))));
        candidates.insert(short_hash, short_hash_candidate);

        for ctx in &selected.execution_contexts {
            let env_detail = if selected.shared_env.is_empty() {
                String::new()
            } else {
                format!(" {}", format_envs(&ctx.env, &selected.shared_env),)
            };
            let ctx_hash = ctx.hash.clone();
            let ctx_candidate =
                CompletionCandidate::new(ctx_hash.as_str()).help(Some(StyledStr::from(format!(
                    "{} ({}){}{}",
                    selected.name, selected.python, pkgs_detail, env_detail
                ))));
            candidates.insert(ctx_hash, ctx_candidate);
        }
    }

    let mut completion_candidates = Vec::new();
    for (_, candidate) in candidates {
        completion_candidates.push(candidate);
    }

    completion_candidates
}

pub struct SelectorCompleter;

impl ValueCompleter for SelectorCompleter {
    fn complete(&self, current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
        complete_selector(current, true)
    }
}

pub struct HashCompleter;

impl ValueCompleter for HashCompleter {
    fn complete(&self, current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
        complete_selector(current, false)
    }
}
