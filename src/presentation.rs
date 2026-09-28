use std::collections::HashMap;

use indexmap::IndexMap;

use crate::venv::{Suite, TestEnvironment, TestRun, pytest_targets};

pub struct EnvironmentPresentation {
    pub name: String,
    pub runs: HashMap<String, RunPresentation>,
}

pub struct RunPresentation {
    pub label: Option<String>,
    pub display_name: String,
    pub short_display_name: String,
    pub differing_env: IndexMap<String, Option<String>>,
}

/// Presentation is derived from the complete suite, even when a selector hides siblings.
pub fn describe_suites(
    suites: &IndexMap<String, Suite>,
) -> HashMap<String, EnvironmentPresentation> {
    let mut presentations = HashMap::new();
    for suite in suites.values() {
        let mut names: Vec<String> = suite
            .environments
            .iter()
            .map(|environment| environment_name(suite, environment))
            .collect();
        disambiguate_environments(&mut names, &suite.environments);
        for (environment, name) in suite.environments.iter().zip(names) {
            let mut run_labels: Vec<Option<String>> = environment
                .runs
                .iter()
                .map(|run| run_label(environment, run))
                .collect();
            disambiguate_runs(&mut run_labels, &environment.runs);
            let runs = environment
                .runs
                .iter()
                .zip(run_labels)
                .map(|(run, label)| {
                    let suffix = label
                        .as_ref()
                        .map_or_else(String::new, |label| format!(" · {label}"));
                    let short = format!("{name} ({}){suffix}", environment.python);
                    let display = format!("{name} · Python {}{suffix}", environment.python);
                    (
                        run.id.clone(),
                        RunPresentation {
                            label,
                            display_name: display,
                            short_display_name: short,
                            differing_env: differing_env(&environment.runs, run),
                        },
                    )
                })
                .collect();
            presentations.insert(
                environment.hash.clone(),
                EnvironmentPresentation {
                    name: format!("{name} · Python {}", environment.python),
                    runs,
                },
            );
        }
    }
    presentations
}

fn environment_name(suite: &Suite, environment: &TestEnvironment) -> String {
    let peers: Vec<_> = suite
        .environments
        .iter()
        .filter(|peer| peer.name == environment.name && peer.python == environment.python)
        .collect();
    if peers.len() == 1 {
        return environment.name.clone();
    }
    let differing: Vec<_> = environment
        .direct_dependencies
        .iter()
        .filter(|dependency| {
            peers
                .iter()
                .any(|peer| !peer.direct_dependencies.contains(dependency))
        })
        .collect();
    if differing.is_empty() {
        return environment.name.clone();
    }
    let own_requirement = differing.iter().position(|dependency| {
        dependency
            .split(['<', '>', '=', '!', '~', '[', ';', ' '])
            .next()
            .is_some_and(|name| name.eq_ignore_ascii_case(&environment.name))
    });
    let mut shown: Vec<&str> = own_requirement
        .map(|index| differing[index].as_str())
        .into_iter()
        .collect();
    shown.extend(
        differing
            .iter()
            .enumerate()
            .filter(|(index, _)| Some(*index) != own_requirement)
            .take(2 - shown.len())
            .map(|(_, dependency)| dependency.as_str()),
    );
    let mut name = if own_requirement.is_some() {
        shown.join(", ")
    } else {
        format!("{} · {}", environment.name, shown.join(", "))
    };
    if differing.len() > 2 {
        name.push_str(" …");
    }
    name
}

fn differing_env(runs: &[TestRun], run: &TestRun) -> IndexMap<String, Option<String>> {
    if runs.len() == 1 {
        return IndexMap::new();
    }
    let mut keys: Vec<_> = runs
        .iter()
        .flat_map(|run| run.env.keys().cloned())
        .collect();
    keys.sort();
    keys.dedup();
    keys.into_iter()
        .filter(|key| {
            runs.iter()
                .any(|peer| peer.env.get(key) != runs[0].env.get(key))
        })
        .map(|key| {
            let value = run.env.get(&key).cloned();
            (key, value)
        })
        .collect()
}

fn run_label(environment: &TestEnvironment, run: &TestRun) -> Option<String> {
    if environment.runs.len() == 1 {
        return None;
    }
    let targets = pytest_targets(&run.command);
    let targets_differ = environment
        .runs
        .iter()
        .any(|peer| pytest_targets(&peer.command) != targets);
    let target = targets_differ.then(|| {
        targets
            .iter()
            .take(2)
            .map(|target| target.rsplit('/').next().unwrap_or(target))
            .collect::<Vec<_>>()
            .join(", ")
    });
    let differences = differing_env(&environment.runs, run);
    let variable = differences
        .iter()
        .find(|(_, value)| value.is_some())
        .or_else(|| differences.first())
        .map(|(key, value)| format!("{key}={}", value.as_deref().unwrap_or("<inherited>")));
    let parts: Vec<_> = [target.filter(|value| !value.is_empty()), variable]
        .into_iter()
        .flatten()
        .collect();
    (!parts.is_empty()).then(|| parts.join(" · "))
}

fn disambiguate_environments(names: &mut [String], environments: &[TestEnvironment]) {
    let mut counts = HashMap::new();
    for (name, environment) in names.iter().zip(environments) {
        *counts
            .entry((name.clone(), environment.python.clone()))
            .or_insert(0_usize) += 1;
    }
    for (name, environment) in names.iter_mut().zip(environments) {
        if counts[&(name.clone(), environment.python.clone())] > 1 {
            *name = format!("{name} [{}]", environment.hash);
        }
    }
}

fn disambiguate_runs(labels: &mut [Option<String>], runs: &[TestRun]) {
    if runs.len() == 1 {
        return;
    }
    let commands_differ = runs.iter().any(|run| run.command != runs[0].command);
    if commands_differ {
        let names: Vec<_> = labels
            .iter()
            .map(|label| label.clone().unwrap_or_default())
            .collect();
        for (index, label) in labels.iter_mut().enumerate() {
            if names.iter().filter(|name| **name == names[index]).count() > 1 {
                let hint = command_hint(runs, &runs[index]);
                *label = Some(
                    label
                        .as_ref()
                        .map_or_else(|| hint.clone(), |name| format!("{name} · {hint}")),
                );
            }
        }
    }
    let names: Vec<_> = labels
        .iter()
        .map(|label| label.clone().unwrap_or_default())
        .collect();
    for (index, label) in labels.iter_mut().enumerate() {
        if names.iter().filter(|name| **name == names[index]).count() > 1 {
            let base = label.as_deref().unwrap_or("run");
            *label = Some(format!("{base} [{}]", runs[index].id));
        }
    }
}

fn command_hint(runs: &[TestRun], run: &TestRun) -> String {
    let tokens = shell_words::split(&run.command).unwrap_or_default();
    let distinct: Vec<_> = tokens
        .iter()
        .filter(|token| {
            token.as_str() != "{cmdargs}"
                && runs.iter().any(|peer| {
                    peer.id != run.id
                        && !shell_words::split(&peer.command)
                            .unwrap_or_default()
                            .contains(token)
                })
        })
        .take(2)
        .map(|token| token.rsplit('/').next().unwrap_or(token).to_string())
        .collect();
    if distinct.is_empty() {
        run.command.clone()
    } else {
        distinct.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(index: usize, command: &str, env: &[(&str, &str)]) -> TestRun {
        TestRun {
            id: format!("abc1234@{index}"),
            index,
            command: command.to_string(),
            env: env
                .iter()
                .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
                .collect(),
        }
    }

    fn environment(hash: &str, dependencies: &[&str], runs: Vec<TestRun>) -> TestEnvironment {
        TestEnvironment {
            suite: "contrib::flask".to_string(),
            hash: hash.to_string(),
            name: "flask".to_string(),
            python: "3.14".to_string(),
            direct_dependencies: dependencies.iter().map(ToString::to_string).collect(),
            runs,
            ..TestEnvironment::empty_for_tests()
        }
    }

    fn descriptions(
        environments: Vec<TestEnvironment>,
    ) -> HashMap<String, EnvironmentPresentation> {
        let suite = Suite {
            id: "contrib::flask".to_string(),
            name: "flask".to_string(),
            patterns: Vec::new(),
            source_files: Vec::new(),
            services: Vec::new(),
            env: IndexMap::new(),
            environments,
        };
        describe_suites(&IndexMap::from([(suite.id.clone(), suite)]))
    }

    #[test]
    fn single_run_has_no_ordinal_or_differing_environment() {
        let descriptions = descriptions(vec![environment(
            "abc1234",
            &["flask"],
            vec![run(0, "pytest tests/contrib/flask", &[("SHARED", "value")])],
        )]);
        let run = &descriptions["abc1234"].runs["abc1234@0"];
        assert_eq!(run.display_name, "flask · Python 3.14");
        assert_eq!(run.label, None);
        assert!(run.differing_env.is_empty());
    }

    #[test]
    fn dependency_variants_use_the_distinguishing_requirement() {
        let descriptions = descriptions(vec![
            environment(
                "abc1234",
                &["flask~=2.0", "pytest"],
                vec![run(0, "pytest tests/contrib/flask", &[])],
            ),
            environment(
                "def5678",
                &["flask~=3.0", "pytest"],
                vec![run(0, "pytest tests/contrib/flask", &[])],
            ),
            environment(
                "ghi9012",
                &["flask", "pytest"],
                vec![run(0, "pytest tests/contrib/flask", &[])],
            ),
        ]);
        assert_eq!(descriptions["abc1234"].name, "flask~=2.0 · Python 3.14");
        assert_eq!(descriptions["def5678"].name, "flask~=3.0 · Python 3.14");
        assert_eq!(descriptions["ghi9012"].name, "flask · Python 3.14");
    }

    #[test]
    fn python_versions_do_not_need_hash_disambiguation() {
        let mut py313 = environment(
            "abc1234",
            &["flask"],
            vec![run(0, "pytest tests/contrib/flask", &[])],
        );
        py313.python = "3.13".to_string();
        let py314 = environment(
            "def5678",
            &["flask"],
            vec![run(0, "pytest tests/contrib/flask", &[])],
        );
        let descriptions = descriptions(vec![py313, py314]);
        assert_eq!(descriptions["abc1234"].name, "flask · Python 3.13");
        assert_eq!(descriptions["def5678"].name, "flask · Python 3.14");
    }

    #[test]
    fn matching_requirement_replaces_redundant_environment_name() {
        let descriptions = descriptions(vec![
            environment(
                "abc1234",
                &["werkzeug>=3", "flask~=3.0"],
                vec![run(0, "pytest tests/contrib/flask", &[])],
            ),
            environment(
                "def5678",
                &["werkzeug<3", "flask~=2.0"],
                vec![run(0, "pytest tests/contrib/flask", &[])],
            ),
        ]);
        assert_eq!(
            descriptions["abc1234"].name,
            "flask~=3.0, werkzeug>=3 · Python 3.14"
        );
    }

    #[test]
    fn indistinguishable_dependency_variants_fall_back_to_hashes() {
        let descriptions = descriptions(vec![
            environment(
                "abc1234",
                &["flask"],
                vec![run(0, "pytest tests/contrib/flask", &[])],
            ),
            environment(
                "def5678",
                &["flask"],
                vec![run(0, "pytest tests/contrib/flask", &[])],
            ),
        ]);
        assert_eq!(
            descriptions["abc1234"].name,
            "flask [abc1234] · Python 3.14"
        );
        assert_eq!(
            descriptions["def5678"].name,
            "flask [def5678] · Python 3.14"
        );
    }

    #[test]
    fn multi_run_labels_show_only_differences() {
        let descriptions = descriptions(vec![environment(
            "abc1234",
            &["flask"],
            vec![
                run(
                    0,
                    "pytest tests/contrib/flask",
                    &[("SHARED", "value"), ("DD_SERVICE", "one")],
                ),
                run(
                    1,
                    "pytest tests/contrib/flask_autopatch",
                    &[("SHARED", "value"), ("DD_SERVICE", "two")],
                ),
            ],
        )]);
        let first = &descriptions["abc1234"].runs["abc1234@0"];
        let second = &descriptions["abc1234"].runs["abc1234@1"];
        assert_eq!(first.label.as_deref(), Some("flask · DD_SERVICE=one"));
        assert_eq!(
            second.label.as_deref(),
            Some("flask_autopatch · DD_SERVICE=two")
        );
        assert_eq!(first.differing_env["DD_SERVICE"], Some("one".to_string()));
        assert!(!first.differing_env.contains_key("SHARED"));
    }

    #[test]
    fn command_only_variants_get_target_labels() {
        let descriptions = descriptions(vec![environment(
            "abc1234",
            &["flask"],
            vec![
                run(0, "pytest tests/contrib/flask", &[("SHARED", "value")]),
                run(
                    1,
                    "python tests/ddtrace_run.py pytest tests/contrib/flask_autopatch",
                    &[("SHARED", "value")],
                ),
            ],
        )]);
        let first = &descriptions["abc1234"].runs["abc1234@0"];
        let second = &descriptions["abc1234"].runs["abc1234@1"];
        assert_eq!(first.label.as_deref(), Some("flask"));
        assert_eq!(second.label.as_deref(), Some("flask_autopatch"));
        assert!(first.differing_env.is_empty());
    }

    #[test]
    fn env_only_variants_and_absent_values_are_labeled() {
        let descriptions = descriptions(vec![environment(
            "abc1234",
            &["flask"],
            vec![
                run(0, "pytest tests/contrib/flask", &[]),
                run(
                    1,
                    "pytest tests/contrib/flask",
                    &[("PYTHONMALLOC", "malloc")],
                ),
            ],
        )]);
        let first = &descriptions["abc1234"].runs["abc1234@0"];
        let second = &descriptions["abc1234"].runs["abc1234@1"];
        assert_eq!(first.label.as_deref(), Some("PYTHONMALLOC=<inherited>"));
        assert_eq!(second.label.as_deref(), Some("PYTHONMALLOC=malloc"));
        assert_eq!(first.differing_env["PYTHONMALLOC"], None);
    }

    #[test]
    fn identical_runs_use_the_stable_selector_only_as_a_last_resort() {
        let descriptions = descriptions(vec![environment(
            "abc1234",
            &["flask"],
            vec![
                run(0, "pytest tests/contrib/flask", &[]),
                run(1, "pytest tests/contrib/flask", &[]),
            ],
        )]);
        assert_eq!(
            descriptions["abc1234"].runs["abc1234@0"].label.as_deref(),
            Some("run [abc1234@0]")
        );
    }
}
