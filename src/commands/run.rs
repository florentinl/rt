use std::{io::IsTerminal, sync::Arc};

use indexmap::IndexMap;

use crate::{
    command::ManagedCommand,
    commands::build::{build_selected_contexts, collect_context_indices},
    config::{RepoConfig, Selector},
    error::{RtError, RtResult},
    progress::{
        MultiplexedProgressLogger, PlainProgressLogger, ProgressLogger, StepContext, StepId, Task,
        TaskRunner, summarize_errors,
    },
    venv::{ExecutionContext, RiotVenv, select_execution_contexts, venv_python_path},
};

pub struct Options {
    pub force_reinstall: bool,
    pub no_editable: bool,
    pub parallel: Option<usize>,
    pub command_override: Option<String>,
    pub cmdargs: Vec<String>,
}

/// Build and execute the command for the given execution context.
///
/// # Errors
///
/// Returns an error if context selection, build, or command execution fails.
pub fn run(
    venvs: IndexMap<String, RiotVenv>,
    repo: &RepoConfig,
    selector: Selector,
    options: &Options,
) -> RtResult<()> {
    let selected = select_execution_contexts(venvs, selector)?;
    let command_override = options.command_override.as_deref();

    for selected_venv in &selected {
        for exc_ctx in &selected_venv.execution_contexts {
            if command_override.is_none() && exc_ctx.command.is_none() {
                return Err(RtError::message(format!(
                    "error: execution context {} has no command configured",
                    exc_ctx.hash
                )));
            }
        }
    }

    build_selected_contexts(
        repo,
        &selected,
        options.force_reinstall,
        options.no_editable,
    )?;

    let sink: Arc<dyn ProgressLogger> = match options.parallel {
        Some(n) if n > 0 && std::io::stderr().is_terminal() => {
            match MultiplexedProgressLogger::new() {
                Ok(logger) => Arc::new(logger),
                Err(_) => Arc::new(PlainProgressLogger::default()),
            }
        }
        _ => Arc::new(PlainProgressLogger::default()),
    };

    run_contexts(
        repo,
        &selected,
        command_override,
        &options.cmdargs,
        options.parallel,
        sink,
    )
}

fn run_contexts(
    repo: &RepoConfig,
    selected: &[RiotVenv],
    command_override: Option<&str>,
    cmdargs: &[String],
    parallelism: Option<usize>,
    sink: Arc<dyn ProgressLogger>,
) -> RtResult<()> {
    let runner = TaskRunner::new(sink).with_parallelism(parallelism);

    let tasks: Vec<Task<'_, RtError>> = collect_context_indices(selected)
        .iter()
        .map(|&(venv_i, exc_i)| {
            let exc_ctx: ExecutionContext = selected[venv_i].execution_contexts[exc_i].clone();
            let label = format!("Execute {}", exc_ctx.hash);
            Task::new(StepId::new(exc_ctx.hash.clone()), label, move |ctx| {
                execute_command(repo, &exc_ctx, command_override, cmdargs, &ctx)
            })
        })
        .collect();

    let errors = runner.run(tasks).map_err(|err| {
        RtError::message(format!("error: could not configure parallelism ({err})"))
    })?;

    if summarize_errors(&errors, "run") {
        return Err(RtError::silent(1));
    }

    Ok(())
}

fn execute_command(
    repo: &RepoConfig,
    exc_ctx: &ExecutionContext,
    command_override: Option<&str>,
    cmdargs: &[String],
    ctx: &StepContext,
) -> RtResult<()> {
    let command_line = {
        let mut command_template = command_override
            .or(exc_ctx.command.as_deref())
            .expect("command presence was validated before building")
            .to_owned();
        if !command_template.contains("{cmdargs}") {
            command_template.push_str(" {cmdargs}");
        }
        command_template.replace("{cmdargs}", &format_cmdargs(cmdargs))
    };

    let status = ManagedCommand::new_uv("run", Arc::clone(&ctx.sink), ctx.step_id.clone())
        .envs(&exc_ctx.env)
        .arg("--no-project")
        .args([
            "--python",
            &venv_python_path(&repo.riot_root, &exc_ctx.hash),
        ])
        .arg("--")
        .args(["sh", "-c", &command_line])
        .status()
        .map_err(|err| {
            RtError::message(format!(
                "error: failed to execute command `{command_line}`: {err}"
            ))
        })?;

    if status.success() {
        Ok(())
    } else {
        Err(RtError::silent_from_status(status))
    }
}

fn format_cmdargs(args: &[String]) -> String {
    if args.is_empty() {
        String::new()
    } else {
        args.iter()
            .map(|arg| escape_cmdarg(arg))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn escape_cmdarg(arg: &str) -> String {
    if arg.is_empty() {
        "''".to_string()
    } else {
        let mut escaped = String::from("'");
        for ch in arg.chars() {
            if ch == '\'' {
                escaped.push_str("'\\''");
            } else {
                escaped.push(ch);
            }
        }
        escaped.push('\'');
        escaped
    }
}
