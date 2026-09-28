use std::collections::HashSet;

use clap_complete::{CompletionCandidate, engine::ValueCompleter};

use crate::{
    config::Selector,
    locate_project_root,
    venv::{compare_python_versions, flatten_suites, load_context, select_test_environments},
};

fn get_suites() -> indexmap::IndexMap<String, crate::venv::Suite> {
    locate_project_root(None)
        .and_then(|root| load_context(&root, None))
        .unwrap_or_default()
}

pub struct PythonCompleter;
impl ValueCompleter for PythonCompleter {
    fn complete(&self, current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
        let Some(hint) = current.to_str() else {
            return Vec::new();
        };
        let mut values: Vec<_> = flatten_suites(&get_suites())
            .into_values()
            .map(|environment| environment.python)
            .filter(|python| python.starts_with(hint))
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        values.sort_by(|left, right| compare_python_versions(left, right));
        values.into_iter().map(CompletionCandidate::new).collect()
    }
}

pub struct NameCompleter;
impl ValueCompleter for NameCompleter {
    fn complete(&self, current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
        let Some(hint) = current.to_str() else {
            return Vec::new();
        };
        select_test_environments(&get_suites(), Selector::Pattern(hint.to_owned()))
            .unwrap_or_default()
            .into_iter()
            .map(|environment| environment.suite)
            .collect::<HashSet<_>>()
            .into_iter()
            .map(CompletionCandidate::new)
            .collect()
    }
}

fn complete_selector(current: &std::ffi::OsStr, names: bool) -> Vec<CompletionCandidate> {
    let Some(hint) = current.to_str() else {
        return Vec::new();
    };
    let mut values = Vec::new();
    for environment in select_test_environments(&get_suites(), Selector::Pattern(hint.to_owned()))
        .unwrap_or_default()
    {
        if names {
            values.push(CompletionCandidate::new(environment.suite.clone()));
        }
        values.push(CompletionCandidate::new(environment.hash.clone()));
        values.extend(
            environment
                .runs
                .into_iter()
                .map(|run| CompletionCandidate::new(run.id)),
        );
    }
    values
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
