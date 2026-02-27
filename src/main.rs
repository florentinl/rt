#![warn(clippy::all)]
#![warn(clippy::pedantic)]
#![warn(clippy::nursery)]
#![allow(clippy::literal_string_with_formatting_args)]

use std::{
    env::args,
    process::{ExitCode, exit},
};

use clap::{CommandFactory, Parser};
use clap_complete::CompleteEnv;
use pyo3::Python;

mod command;
mod commands;
mod completion;
mod config;
mod constants;
mod display;
mod fake_ruamel_yaml;
mod progress;
mod ui;
mod venv;

use crate::{
    config::{RepoConfig, RunConfig, Selector, load_rt_toml},
    venv::{RiotVenv, get_context},
};
use clap::{Subcommand, ValueHint};
use clap_complete::engine::ArgValueCompleter;
use indexmap::IndexMap;
use pyo3::exceptions::PySystemExit;
use pyo3::prelude::*;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "rt",
    version,
    about = "CLI proxy exported from the rt native extension."
)]
struct Cli {
    /// Path to the riotfile to use. Falls back to discovery if omitted.
    #[arg(short, long, value_name = "PATH", add = ValueHint::FilePath)]
    pub file: Option<PathBuf>,
    #[arg(short, long, value_name = "PATH", add = ValueHint::DirPath)]
    pub riot_root: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List fully specified virtual environments with inherited configuration.
    #[command(alias = "ls")]
    List {
        /// Print only the unique hashes of the normalized virtual environments.
        #[arg(long = "hash-only")]
        hash_only: bool,
        /// Output the selected environments as JSON instead of a hierarchy.
        #[arg(long = "json")]
        json: bool,
        /// Optional regular expression to match venv names.
        #[arg(
            value_name = "NAME_PATTERN",
            add = ArgValueCompleter::new(completion::NameCompleter)
        )]
        pattern: Option<String>,
        /// Filter venvs to specific Python versions.
        #[arg(
            short = 'p',
            long = "python",
            value_name = "PYTHON",
            add = ArgValueCompleter::new(completion::PythonCompleter)
        )]
        python: Option<Vec<String>>,
    },
    /// Show the normalized configuration for a venv or execution context.
    Describe {
        /// Execution or venv hash.
        #[arg(
            value_name = "HASH",
            add = ArgValueCompleter::new(completion::HashCompleter)
        )]
        hash: String,
    },
    /// Build the virtual environment for execution contexts matched by the selector.
    Build {
        /// Force reinstalling cached dependencies before building.
        #[arg(long = "force-reinstall")]
        force_reinstall: bool,
        /// Filter venvs to specific Python versions.
        #[arg(
            short = 'p',
            long = "python",
            value_name = "PYTHON",
            add = ArgValueCompleter::new(completion::PythonCompleter)
        )]
        python: Option<Vec<String>>,
        /// Selector interpreted as execution context hash, venv hash, or name regex (in that order).
        #[arg(
            value_name = "PATTERN",
            required = true,
            add = ArgValueCompleter::new(completion::SelectorCompleter)
        )]
        pattern: Option<String>,
    },
    /// Build and execute the command for execution contexts matched by the selector.
    Run {
        /// Force reinstalling cached dependencies before running.
        #[arg(long = "force-reinstall")]
        force_reinstall: bool,
        /// Run in parallel (optionally specify worker count).
        #[arg(
            long = "parallel",
            value_name = "N",
            num_args = 0..=1,
            default_missing_value = "10"
        )]
        parallel: Option<usize>,
        /// Override the execution context command template.
        #[arg(long = "command", value_name = "COMMAND")]
        command_override: Option<String>,
        /// Filter venvs to specific Python versions.
        #[arg(
            short = 'p',
            long = "python",
            value_name = "PYTHON",
            add = ArgValueCompleter::new(completion::PythonCompleter)
        )]
        python: Option<Vec<String>>,
        /// Selector interpreted as execution context hash, venv hash, or name regex (in that order).
        #[arg(
            value_name = "PATTERN",
            required = true,
            add = ArgValueCompleter::new(completion::SelectorCompleter)
        )]
        pattern: String,
        /// Arguments forwarded to the execution context command after `--`.
        #[arg(value_name = "CMDARGS", trailing_var_arg = true)]
        cmdargs: Vec<String>,
    },
    /// Build the virtual environment and start a shell with it activated.
    Shell {
        /// Execution or venv hash.
        #[arg(
            value_name = "HASH",
            add = ArgValueCompleter::new(completion::HashCompleter)
        )]
        hash: String,
        /// Force reinstalling cached dependencies before opening the shell.
        #[arg(long = "force-reinstall")]
        force_reinstall: bool,
    },
    /// Build the virtual environment and print the activation script path.
    Activate {
        /// Execution or venv hash.
        #[arg(
            value_name = "HASH",
            add = ArgValueCompleter::new(completion::HashCompleter)
        )]
        hash: String,
        /// Force reinstalling cached dependencies before preparing the environment.
        #[arg(long = "force-reinstall")]
        force_reinstall: bool,
    },
    /// Build the virtual environment and link it as .venv in the riotfile directory.
    Switch {
        /// Execution or venv hash.
        #[arg(
            value_name = "HASH",
            add = ArgValueCompleter::new(completion::HashCompleter)
        )]
        hash: String,
        /// Force reinstalling cached dependencies before preparing the environment.
        #[arg(long = "force-reinstall")]
        force_reinstall: bool,
    },
    /// Remove all cached virtual environments while keeping compiled requirements.
    Clean,
}

#[derive(Subcommand)]
enum VscodeCommands {
    /// Remove VS Code configuration set by rt.
    Clear,
}

/// Dispatch the selected CLI command.
///
/// # Errors
///
/// Returns an error if command execution fails.
fn run_command(
    riot_venvs: IndexMap<String, RiotVenv>,
    cli: Cli,
    repo: &RepoConfig,
) -> PyResult<()> {
    match cli.command {
        Commands::List {
            hash_only,
            json,
            pattern,
            python,
        } => {
            if hash_only && json {
                eprintln!("error: --hash-only and --json cannot be used together.");
                return Err(PyErr::new::<PySystemExit, _>(2));
            }
            let selector = Selector::Generic { python, pattern };
            commands::list::run(riot_venvs, repo, selector, hash_only, json)
        }
        Commands::Describe { hash } => commands::describe::run(riot_venvs, repo, hash),
        Commands::Build {
            force_reinstall,
            pattern,
            python,
        } => commands::build::run(
            riot_venvs,
            repo,
            Selector::Generic { python, pattern },
            force_reinstall,
        ),
        Commands::Run {
            force_reinstall,
            parallel,
            command_override,
            python,
            pattern,
            cmdargs,
        } => {
            let run_config = RunConfig {
                command_override,
                cmdargs,
                action_label: "Execute".to_string(),
            };
            commands::run::run(
                riot_venvs,
                repo,
                Selector::Generic {
                    python,
                    pattern: Some(pattern),
                },
                force_reinstall,
                parallel,
                &run_config,
            )
        }
        Commands::Shell {
            hash,
            force_reinstall,
        } => commands::shell::run(riot_venvs, repo, &hash, force_reinstall),
        Commands::Activate {
            hash,
            force_reinstall,
        } => commands::activate::run(riot_venvs, repo, &hash, force_reinstall),
        Commands::Switch {
            hash,
            force_reinstall,
        } => commands::switch::run(riot_venvs, repo, &hash, force_reinstall),
        Commands::Clean => commands::clean::run(&repo.riot_root),
    }
}

#[must_use]
fn locate_riotfile(riotfile_arg: Option<&PathBuf>) -> PathBuf {
    if let Some(path) = riotfile_arg {
        if !path.is_file() {
            eprintln!(
                "error: specified riotfile {} does not exist.",
                path.display()
            );
            exit(1);
        }
        return path.to_owned();
    }

    let Ok(mut dir) = std::env::current_dir() else {
        exit(1);
    };

    loop {
        let candidate = dir.join("riotfile.py");
        if candidate.is_file() {
            return candidate;
        }

        if dir.join(".git").is_dir() {
            break;
        }

        if !dir.pop() {
            break;
        }
    }
    eprintln!(
        "error: riotfile.py not found in the current workspace (searched up to the git repository root)."
    );
    exit(1);
}

#[must_use]
fn locate_riotroot(riotfile_path: &Path, riot_root_path: Option<&PathBuf>) -> PathBuf {
    let path = riot_root_path.map_or_else(
        || {
            riotfile_path
                .parent()
                .and_then(|parent| parent.exists().then_some(parent.join(".riot")))
        },
        |path| {
            path.parent()
                .is_some_and(Path::exists)
                .then_some(path.clone())
        },
    );

    let Some(path) = path else {
        eprintln!("error: could not create riot root directory");
        exit(1);
    };
    path
}

fn main() -> ExitCode {
    if let Ok(value) = std::env::var("RT_IS_UV_NOW")
        && value == "true"
    {
        return unsafe { uv::main(args()) };
    }

    CompleteEnv::with_factory(Cli::command).complete();

    let cli = Cli::parse();

    let riotfile_path = locate_riotfile(cli.file.as_ref());
    let riot_root = locate_riotroot(&riotfile_path, cli.riot_root.as_ref());

    Python::initialize();
    Python::attach(|py| {
        py.import("gc").unwrap().call_method0("disable").unwrap();
    });
    let riot_venvs = Python::attach(|py| get_context(py, &riotfile_path));

    let (build_env, run_env) = load_rt_toml(&riotfile_path);
    let repo_config = RepoConfig::load(riotfile_path, riot_root, build_env, run_env);

    if run_command(riot_venvs, cli, &repo_config).is_err() {
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
