use std::{io::IsTerminal, sync::Arc};

use indexmap::IndexMap;

use crate::{
    command::ManagedCommand,
    commands::build::build_selected_environments,
    config::{RepoConfig, Selector},
    constants::RUNTIME_CONFIG,
    error::{RtError, RtResult},
    progress::{
        MultiplexedProgressLogger, PlainProgressLogger, ProgressLogger, StepContext, StepId, Task,
        TaskRunner, summarize_errors,
    },
    venv::{Suite, TestEnvironment, TestRun, profile_path, select_test_environments},
};

pub struct Options {
    pub force_reinstall: bool,
    pub no_editable: bool,
    pub parallel: Option<usize>,
    pub command_override: Option<String>,
    pub cmdargs: Vec<String>,
}

pub fn run(
    suites: &IndexMap<String, Suite>,
    repo: &RepoConfig,
    selector: Selector,
    options: &Options,
) -> RtResult<()> {
    let selected = select_test_environments(suites, selector)?;
    build_selected_environments(
        repo,
        &selected,
        options.force_reinstall,
        options.no_editable,
    )?;
    let sink: Arc<dyn ProgressLogger> = match options.parallel {
        Some(value) if value > 0 && std::io::stderr().is_terminal() => {
            MultiplexedProgressLogger::new().map_or_else(
                |_| Arc::new(PlainProgressLogger::default()) as Arc<_>,
                |logger| Arc::new(logger) as Arc<_>,
            )
        }
        _ => Arc::new(PlainProgressLogger::default()),
    };
    run_profiles(
        repo,
        &selected,
        options.command_override.as_deref(),
        &options.cmdargs,
        options.parallel,
        sink,
    )
}

fn run_profiles(
    repo: &RepoConfig,
    selected: &[TestEnvironment],
    override_command: Option<&str>,
    cmdargs: &[String],
    parallelism: Option<usize>,
    sink: Arc<dyn ProgressLogger>,
) -> RtResult<()> {
    let runner = TaskRunner::new(sink).with_parallelism(parallelism);
    let tasks: Vec<_> = selected
        .iter()
        .flat_map(|environment| environment.runs.iter().map(move |run| (environment, run)))
        .map(|(environment, run)| {
            let label = format!("Execute {}", run.id);
            Task::new(StepId::new(run.id.clone()), label, move |context| {
                execute_command(repo, environment, run, override_command, cmdargs, &context)
            })
        })
        .collect();
    let errors = runner.run(tasks).map_err(|error| {
        RtError::message(format!("error: could not configure parallelism ({error})"))
    })?;
    if summarize_errors(&errors, "run") {
        return Err(RtError::silent(1));
    }
    Ok(())
}

fn execute_command(
    repo: &RepoConfig,
    environment: &TestEnvironment,
    run: &TestRun,
    override_command: Option<&str>,
    cmdargs: &[String],
    context: &StepContext,
) -> RtResult<()> {
    let mut template = override_command.unwrap_or(&run.command).to_owned();
    if !template.contains("{cmdargs}") {
        template.push_str(" {cmdargs}");
    }
    let command_line = template.replace("{cmdargs}", &format_cmdargs(cmdargs));
    let profile = profile_path(&repo.cache_root, &environment.hash, run.index);
    let status = ManagedCommand::new_uv("run", Arc::clone(&context.sink), context.step_id.clone())
        .current_dir(&repo.project_root)
        .env("RT_CONTEXT_PATH", profile.join(RUNTIME_CONFIG))
        .envs(&environment.suite_env)
        .envs(&run.env)
        .arg("--no-project")
        .arg("--python")
        .arg(profile.join("bin/python"))
        .arg("--")
        .args(["sh", "-c", &command_line])
        .status()
        .map_err(|error| {
            RtError::message(format!(
                "error: failed to execute command `{command_line}`: {error}"
            ))
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(RtError::silent_from_status(status))
    }
}

fn format_cmdargs(args: &[String]) -> String {
    args.iter()
        .map(|arg| format!("'{}'", arg.replace('\'', "'\\''")))
        .collect::<Vec<_>>()
        .join(" ")
}
