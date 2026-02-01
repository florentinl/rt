use std::{io::IsTerminal, sync::Arc};

use pyo3::{exceptions::PySystemExit, PyErr, PyResult, Python};

use crate::{
    command::ManagedCommand,
    commands::build::{build_selected_contexts, collect_context_indices},
    config::{RepoConfig, RunConfig, Selector},
    progress::{
        summarize_errors, MultiplexedProgressLogger, PlainProgressLogger, ProgressLogger,
        StepContext, StepId, StepOutcome, Task, TaskRunner,
    },
    venv::{select_execution_contexts, venv_python_path, ExecutionContext, RiotVenv},
};
/// Build and execute the command for the given execution context.
pub fn run(
    py: Python<'_>,
    repo: &RepoConfig,
    selector: Selector,
    force_reinstall: bool,
    parallel: Option<usize>,
    run_config: &RunConfig,
) -> PyResult<()> {
    let selected = select_execution_contexts(py, &repo.riotfile_path, selector)?;

    for selected_venv in &selected {
        for exc_ctx in &selected_venv.execution_contexts {
            if run_config.command_override.is_none() && exc_ctx.command.is_none() {
                eprintln!(
                    "error: execution context {} has no command configured",
                    exc_ctx.hash
                );
                return Err(PyErr::new::<PySystemExit, _>(1));
            }
        }
    }

    build_selected_contexts(repo, &selected, force_reinstall)?;

    let sink: Arc<dyn ProgressLogger> = match parallel {
        Some(n) if n > 0 && std::io::stderr().is_terminal() => {
            match MultiplexedProgressLogger::new() {
                Ok(logger) => Arc::new(logger),
                Err(_) => Arc::new(PlainProgressLogger::default()),
            }
        }
        _ => Arc::new(PlainProgressLogger::default()),
    };

    run_contexts(repo, &selected, run_config, parallel, sink)
}

fn run_contexts(
    repo: &RepoConfig,
    selected: &[RiotVenv],
    run_config: &RunConfig,
    parallelism: Option<usize>,
    sink: Arc<dyn ProgressLogger>,
) -> PyResult<()> {
    let runner = TaskRunner::new(sink).with_parallelism(parallelism);

    let tasks: Vec<Task<'_, PyErr>> = collect_context_indices(selected)
        .iter()
        .map(|&(venv_i, exc_i)| {
            let exc_ctx: ExecutionContext = selected[venv_i].execution_contexts[exc_i].clone();
            let label = format!("{} {}", run_config.action_label, exc_ctx.hash);
            Task::new(StepId::new(exc_ctx.hash.clone()), label, move |ctx| {
                execute_command(repo, &exc_ctx, run_config, &ctx)
            })
        })
        .collect();

    let errors = runner.run(tasks).map_err(|err| {
        eprintln!("error: could not configure parallelism ({err})");
        PyErr::new::<PySystemExit, _>(1)
    })?;

    if summarize_errors(&errors, "run") {
        return Err(PyErr::new::<PySystemExit, _>(1));
    }

    Ok(())
}

fn execute_command(
    repo: &RepoConfig,
    exc_ctx: &ExecutionContext,
    run_config: &RunConfig,
    ctx: &StepContext,
) -> PyResult<StepOutcome> {
    let command_line = {
        let exc_ctx: &ExecutionContext = exc_ctx;
        let mut command_template = run_config.command_override.as_ref().map_or_else(
            || exc_ctx.command.as_ref().unwrap().clone(),
            std::clone::Clone::clone,
        );
        if !command_template.contains("{cmdargs}") {
            command_template.push_str(" {cmdargs}");
        }
        command_template.replace("{cmdargs}", &format_cmdargs(&run_config.cmdargs))
    };

    let status = ManagedCommand::new_uv("run", Arc::clone(&ctx.sink), ctx.step_id.clone())
        .envs(&exc_ctx.env)
        .envs(repo.run_env.as_ref())
        .arg("--no-project")
        .args([
            "--python",
            &venv_python_path(&repo.riot_root, &exc_ctx.hash),
        ])
        .arg("--")
        .args(["sh", "-c", &command_line])
        .status()
        .map_err(|err| {
            eprintln!("error: failed to execute command `{command_line}`: {err}");
            PyErr::new::<PySystemExit, _>(1)
        })?;

    status
        .success()
        .then_some(StepOutcome::Done)
        .ok_or_else(|| PyErr::new::<PySystemExit, _>(status.code().unwrap_or(1)))
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
