use std::{
    cmp::Ordering,
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};

use fancy_regex::Regex;
use indexmap::{IndexMap, IndexSet};
use sha2::{Digest, Sha256};

use crate::{
    config::Selector,
    constants::{ENVIRONMENTS_DIR, PROFILES_DIR},
    error::{RtError, RtResult},
};

#[derive(Clone, Debug)]
pub struct Suite {
    pub id: String,
    pub name: String,
    pub patterns: Vec<String>,
    pub source_files: Vec<String>,
    pub services: Vec<String>,
    pub env: IndexMap<String, String>,
    pub environments: Vec<TestEnvironment>,
}

#[derive(Clone, Debug)]
pub struct TestEnvironment {
    pub suite: String,
    pub hash: String,
    pub name: String,
    pub integration_name: String,
    pub python: String,
    pub direct_dependencies: Vec<String>,
    pub resolved_pkgs: IndexMap<String, String>,
    pub display_pkgs: IndexMap<String, String>,
    pub lockfile: String,
    pub services: Vec<String>,
    pub suite_env: IndexMap<String, String>,
    pub patterns: Vec<String>,
    pub runs: Vec<TestRun>,
}

impl TestEnvironment {
    #[cfg(test)]
    #[must_use]
    pub(crate) fn empty_for_tests() -> Self {
        Self {
            suite: String::new(),
            hash: String::new(),
            name: String::new(),
            integration_name: String::new(),
            python: String::new(),
            direct_dependencies: Vec::new(),
            resolved_pkgs: IndexMap::new(),
            display_pkgs: IndexMap::new(),
            lockfile: String::new(),
            services: Vec::new(),
            suite_env: IndexMap::new(),
            patterns: Vec::new(),
            runs: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestRun {
    pub id: String,
    pub index: usize,
    pub command: String,
    pub env: IndexMap<String, String>,
}

#[must_use]
pub fn environment_path(cache_root: &Path, hash: &str) -> PathBuf {
    cache_root.join(ENVIRONMENTS_DIR).join(hash)
}

#[must_use]
pub fn environment_python_path(cache_root: &Path, hash: &str) -> PathBuf {
    environment_path(cache_root, hash).join(if cfg!(windows) {
        "Scripts/python.exe"
    } else {
        "bin/python"
    })
}

#[must_use]
pub fn profile_path(cache_root: &Path, environment_hash: &str, index: usize) -> PathBuf {
    cache_root
        .join(PROFILES_DIR)
        .join(environment_hash)
        .join(index.to_string())
}

#[must_use]
pub fn profile_fingerprint(
    environment: &TestEnvironment,
    run: &TestRun,
    no_editable: bool,
) -> String {
    let payload = serde_json::to_vec(&serde_json::json!({
        "rt_version": crate::constants::RT_VERSION,
        "no_editable": no_editable,
        "python": environment.python,
        "direct_dependencies": environment.direct_dependencies,
        "resolved_packages": environment.resolved_pkgs,
        "integration_name": environment.integration_name,
        "suite_env": environment.suite_env,
        "services": environment.services,
        "command": run.command,
        "run_env": run.env,
    }))
    .expect("runtime profile inputs are serializable");
    let digest = Sha256::digest(payload);
    digest.iter().fold(
        String::with_capacity(digest.len() * 2),
        |mut fingerprint, byte| {
            write!(&mut fingerprint, "{byte:02x}")
                .expect("writing a profile fingerprint cannot fail");
            fingerprint
        },
    )
}

pub fn load_context(
    project_root: &Path,
    _cache_root: Option<&Path>,
) -> RtResult<IndexMap<String, Suite>> {
    crate::suitespec::load(project_root)
}

pub fn flatten_suites(suites: &IndexMap<String, Suite>) -> IndexMap<String, TestEnvironment> {
    suites
        .values()
        .flat_map(|suite| suite.environments.iter().cloned())
        .map(|environment| (environment.hash.clone(), environment))
        .collect()
}

pub fn load_resolved_packages(lockfile: &Path) -> IndexMap<String, String> {
    fs::read_to_string(lockfile)
        .map_or_else(|_| IndexMap::new(), |content| parse_lockfile(&content))
}

fn parse_lockfile(content: &str) -> IndexMap<String, String> {
    let mut resolved = IndexMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('-') {
            continue;
        }
        let line = line.split('#').next().unwrap_or(line).trim();
        let line = line.split(';').next().unwrap_or(line).trim();
        if let Some((name_part, version)) = line.split_once("==") {
            let name = name_part.split('[').next().unwrap_or(name_part).trim();
            if !name.is_empty() && !version.is_empty() {
                resolved.insert(name.to_lowercase().replace('_', "-"), version.to_owned());
            }
        }
    }
    resolved
}

pub fn requirement_name(requirement: &str) -> String {
    let end = requirement
        .find(['<', '>', '=', '!', '~', '@', ';'])
        .unwrap_or(requirement.len());
    requirement[..end]
        .split('[')
        .next()
        .unwrap_or(&requirement[..end])
        .trim()
        .to_lowercase()
        .replace(['_', '.'], "-")
}

pub fn build_display_packages(
    direct: &[String],
    resolved: &IndexMap<String, String>,
) -> IndexMap<String, String> {
    let mut display = IndexMap::new();
    let mut direct_names = IndexSet::new();
    for requirement in direct {
        let name = requirement_name(requirement);
        direct_names.insert(name.clone());
        let constraint = requirement.get(name.len()..).unwrap_or_default().trim();
        let value = resolved.get(&name).map_or_else(
            || {
                if constraint.is_empty() {
                    "(latest)".to_owned()
                } else {
                    format!("({constraint})")
                }
            },
            |version| {
                if constraint.is_empty() {
                    format!("{version} (latest)")
                } else {
                    format!("{version} ({constraint})")
                }
            },
        );
        display.insert(name, value);
    }
    for (name, version) in resolved {
        if !direct_names.contains(name) {
            display.insert(name.clone(), version.clone());
        }
    }
    display
}

#[must_use]
pub fn pytest_args(command: &str) -> Vec<String> {
    let Ok(tokens) = shell_words::split(command) else {
        return Vec::new();
    };
    let Some(index) = tokens.iter().position(|token| token == "pytest") else {
        return Vec::new();
    };
    tokens
        .into_iter()
        .skip(index + 1)
        .filter(|token| token != "{cmdargs}")
        .collect()
}

#[must_use]
pub fn pytest_targets(command: &str) -> Vec<String> {
    pytest_args(command)
        .into_iter()
        .filter(|token| !token.starts_with('-'))
        .filter(|token| {
            let path = token.split("::").next().unwrap_or(token);
            path.starts_with("tests/")
                || path.starts_with("benchmarks/")
                || Path::new(path)
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("py"))
        })
        .collect()
}

#[must_use]
pub fn compare_python_versions(lhs: &str, rhs: &str) -> Ordering {
    let parse = |value: &str| {
        value
            .split('.')
            .map(str::parse::<u32>)
            .collect::<Result<Vec<_>, _>>()
    };
    match (parse(lhs), parse(rhs)) {
        (Ok(mut left), Ok(mut right)) => {
            let length = left.len().max(right.len());
            left.resize(length, 0);
            right.resize(length, 0);
            left.cmp(&right)
        }
        _ => lhs.cmp(rhs),
    }
}

fn pattern_matches_path(pattern: &str, target: &str) -> bool {
    let target = target.split("::").next().unwrap_or(target);
    if !pattern.contains(['*', '?']) {
        let pattern = pattern.trim_end_matches('/');
        return target.starts_with(pattern) || pattern.starts_with(target);
    }
    let pattern: Vec<_> = pattern.chars().collect();
    let target: Vec<_> = target.chars().collect();
    let mut matched = vec![vec![false; target.len() + 1]; pattern.len() + 1];
    matched[0][0] = true;
    for index in 1..=pattern.len() {
        if pattern[index - 1] == '*' {
            matched[index][0] = matched[index - 1][0];
        }
    }
    for pattern_index in 1..=pattern.len() {
        for target_index in 1..=target.len() {
            matched[pattern_index][target_index] = match pattern[pattern_index - 1] {
                '*' => {
                    matched[pattern_index - 1][target_index]
                        || matched[pattern_index][target_index - 1]
                }
                '?' => matched[pattern_index - 1][target_index - 1],
                value => {
                    value == target[target_index - 1]
                        && matched[pattern_index - 1][target_index - 1]
                }
            };
        }
    }
    matched[pattern.len()][target.len()]
}

fn matches_test(environment: &TestEnvironment, target: &str) -> bool {
    let included = environment
        .patterns
        .iter()
        .filter(|pattern| !pattern.starts_with('!'))
        .any(|pattern| pattern_matches_path(pattern, target));
    let excluded = environment
        .patterns
        .iter()
        .filter_map(|pattern| pattern.strip_prefix('!'))
        .any(|pattern| pattern_matches_path(pattern, target));
    included && !excluded
}

fn run_environment_hash(id: &str) -> Option<&str> {
    let (hash, index) = id.split_once('@')?;
    (!hash.is_empty() && index.parse::<usize>().is_ok()).then_some(hash)
}

pub fn select_test_environments(
    suites: &IndexMap<String, Suite>,
    selector: Selector,
) -> RtResult<Vec<TestEnvironment>> {
    let (pattern, python, test) = match selector {
        Selector::Pattern(pattern) => (pattern, None, None),
        Selector::Generic {
            python,
            pattern,
            test,
        } => (pattern.unwrap_or_default(), python, test),
    };
    let mut environments = flatten_suites(suites);
    if let Some(python) = python {
        let selected: IndexSet<_> = python.into_iter().collect();
        environments.retain(|_, environment| selected.contains(&environment.python));
    }
    if let Some(environment) = environments.get(&pattern).cloned() {
        if test
            .as_ref()
            .is_none_or(|target| matches_test(&environment, target))
        {
            return Ok(vec![environment]);
        }
        return Ok(Vec::new());
    }
    if let Some(hash) = run_environment_hash(&pattern)
        && let Some(mut environment) = environments.get(hash).cloned()
    {
        environment.runs.retain(|run| run.id == pattern);
        if environment.runs.is_empty() {
            return Ok(Vec::new());
        }
        return Ok(
            if test
                .as_ref()
                .is_none_or(|target| matches_test(&environment, target))
            {
                vec![environment]
            } else {
                Vec::new()
            },
        );
    }
    if suites.contains_key(&pattern) {
        let mut selected: Vec<_> = environments
            .values()
            .filter(|environment| environment.suite == pattern)
            .cloned()
            .collect();
        if let Some(target) = &test {
            selected.retain(|environment| matches_test(environment, target));
        }
        return Ok(selected);
    }
    let regex = Regex::new(&pattern).map_err(|error| {
        RtError::message(format!("error: invalid environment pattern: {error}"))
    })?;
    let mut selected = Vec::new();
    for environment in environments.into_values() {
        let matched = regex.is_match(&environment.name).map_err(|error| {
            RtError::message(format!(
                "error: failed to evaluate environment pattern: {error}"
            ))
        })? || regex.is_match(&environment.suite).map_err(|error| {
            RtError::message(format!("error: failed to evaluate suite pattern: {error}"))
        })?;
        if matched
            && test
                .as_ref()
                .is_none_or(|target| matches_test(&environment, target))
        {
            selected.push(environment);
        }
    }
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::{
        Suite, TestEnvironment, build_display_packages, matches_test, parse_lockfile,
        profile_fingerprint, pytest_args, pytest_targets, requirement_name,
        select_test_environments,
    };
    use crate::config::Selector;
    use indexmap::IndexMap;

    #[test]
    fn lockfile_packages_include_transitive_dependencies() {
        let resolved = parse_lockfile("pytest==8.3.3\ncoverage[toml]==7.6.8  # via pytest-cov\n");
        assert_eq!(resolved.get("pytest"), Some(&"8.3.3".to_owned()));
        assert_eq!(resolved.get("coverage"), Some(&"7.6.8".to_owned()));
    }

    #[test]
    fn direct_dependency_tooltips_keep_constraints() {
        let resolved = IndexMap::from([("pytest".to_owned(), "8.3.3".to_owned())]);
        let display = build_display_packages(&["pytest>=8,<9".to_owned()], &resolved);
        assert_eq!(display["pytest"], "8.3.3 (>=8,<9)");
        assert_eq!(requirement_name("coverage[toml]>=7"), "coverage");
    }

    #[test]
    fn pytest_arguments_preserve_suite_flags_and_remove_placeholder() {
        let command = "python -m pytest --ddtrace --no-cov -p no:flaky -c tests/pytest.ini tests/contrib/flask {cmdargs}";
        assert_eq!(
            pytest_args(command),
            [
                "--ddtrace",
                "--no-cov",
                "-p",
                "no:flaky",
                "-c",
                "tests/pytest.ini",
                "tests/contrib/flask"
            ]
        );
        assert_eq!(
            pytest_targets(command),
            ["tests/pytest.ini", "tests/contrib/flask"]
        );
    }

    #[test]
    fn suite_patterns_honor_exclusions() {
        let mut environment = TestEnvironment::empty_for_tests();
        environment.patterns = vec![
            "ddtrace/appsec/**".to_owned(),
            "!ddtrace/appsec/ai_guard/*".to_owned(),
        ];
        assert!(matches_test(
            &environment,
            "ddtrace/appsec/_asm_request_context.py"
        ));
        assert!(!matches_test(
            &environment,
            "ddtrace/appsec/ai_guard/__init__.py"
        ));
    }

    #[test]
    fn exact_suite_selection_honors_python_filter() {
        let mut py311 = TestEnvironment::empty_for_tests();
        py311.hash = "py311".to_owned();
        py311.suite = "unit".to_owned();
        py311.python = "3.11".to_owned();
        let mut py312 = py311.clone();
        py312.hash = "py312".to_owned();
        py312.python = "3.12".to_owned();
        let suites = IndexMap::from([(
            "unit".to_owned(),
            Suite {
                id: "unit".to_owned(),
                name: "unit".to_owned(),
                patterns: Vec::new(),
                source_files: Vec::new(),
                services: Vec::new(),
                env: IndexMap::new(),
                environments: vec![py311, py312],
            },
        )]);

        let selected = select_test_environments(
            &suites,
            Selector::Generic {
                python: Some(vec!["3.12".to_owned()]),
                pattern: Some("unit".to_owned()),
                test: None,
            },
        )
        .unwrap();

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].python, "3.12");
    }

    #[test]
    fn profile_fingerprint_tracks_locked_dependencies_and_run_inputs() {
        let mut environment = TestEnvironment::empty_for_tests();
        environment.python = "3.12".to_owned();
        environment
            .resolved_pkgs
            .insert("pytest".to_owned(), "8.4.1".to_owned());
        let run = super::TestRun {
            id: "abc1234@0".to_owned(),
            index: 0,
            command: "pytest tests/unit".to_owned(),
            env: IndexMap::new(),
        };
        let initial = profile_fingerprint(&environment, &run, false);

        environment
            .resolved_pkgs
            .insert("pytest".to_owned(), "8.4.2".to_owned());

        assert_ne!(initial, profile_fingerprint(&environment, &run, false));
        assert_ne!(initial, profile_fingerprint(&environment, &run, true));
    }
}
