#![forbid(unsafe_code)]
#![warn(clippy::all)]
#![warn(clippy::pedantic)]
#![warn(clippy::nursery)]
#![allow(clippy::literal_string_with_formatting_args)]

pub mod command;
mod commands;
mod completion;
mod config;
mod constants;
pub mod display;
pub mod progress;
mod ui;
mod venv;

use crate::config::{RepoConfig, RunConfig, Selector};
use clap::{CommandFactory, Parser, Subcommand, ValueHint};
use clap_complete::Shell;
use clap_complete::{engine::ArgValueCompleter, CompleteEnv};
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
    file: Option<PathBuf>,
    #[arg(short, long, value_name = "PATH", add = ValueHint::DirPath)]
    riot_root: Option<PathBuf>,
    #[command(subcommand)]
    command: Commands,
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
    /// Completions
    Completions {
        #[arg(value_name = "SHELL")]
        shell: Shell,
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
    /// Remove all cached virtual environments while keeping compiled requirements.
    Clean,
}

#[derive(Subcommand)]
enum VscodeCommands {
    /// Remove VS Code configuration set by rt.
    Clear,
}

fn run_command(py: Python<'_>, cli: Cli, repo: &RepoConfig) -> PyResult<()> {
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
            commands::list::run(py, repo, selector, hash_only, json)
        }
        Commands::Describe { hash } => commands::describe::run(py, repo, hash),
        Commands::Build {
            force_reinstall,
            pattern,
            python,
        } => commands::build::run(
            py,
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
                py,
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
        } => commands::shell::run(py, repo, &hash, force_reinstall),
        Commands::Activate {
            hash,
            force_reinstall,
        } => commands::activate::run(py, repo, &hash, force_reinstall),
        Commands::Clean => commands::clean::run(&repo.riot_root),
        Commands::Completions { .. } => unreachable!(),
    }
}

fn locate_git_marker(dir: &Path) -> bool {
    dir.join(".git").exists()
}

fn locate_riotfile(start_dir: Option<PathBuf>) -> Option<PathBuf> {
    let mut dir = match start_dir {
        Some(path) => path,
        None => match std::env::current_dir() {
            Ok(path) => path,
            Err(_) => return None,
        },
    };

    loop {
        let candidate = dir.join("riotfile.py");
        if candidate.is_file() {
            return Some(candidate);
        }

        if locate_git_marker(&dir) {
            break;
        }

        if !dir.pop() {
            break;
        }
    }

    None
}

fn locate_riotroot(riotfile_path: &Path, riot_root_path: Option<&PathBuf>) -> Option<PathBuf> {
    if let Some(path) = riot_root_path {
        path.parent()?.exists().then_some(path.clone())
    } else {
        riotfile_path
            .parent()
            .and_then(|parent| parent.exists().then_some(parent.join(".riot")))
    }
}

fn prepare_cli_args(py: Python<'_>) -> PyResult<Vec<String>> {
    let sys = py.import("sys")?;
    let argv: Vec<String> = sys.getattr("argv")?.extract()?;

    let mut filtered_args = Vec::with_capacity(argv.len().max(1));
    filtered_args.push("rt".to_string());

    let mut skip_next = false;
    for (idx, arg) in argv.into_iter().enumerate() {
        if idx == 0 {
            // argv[0] is either the script path or python executable - omit.
            continue;
        }

        if skip_next {
            skip_next = false;
            continue;
        }

        if arg == "-m" {
            // Skip the module selector and the following module name.
            skip_next = true;
            continue;
        }

        if filtered_args.len() == 1 {
            let path = Path::new(&arg);
            if let Some(name) = path.file_stem().and_then(|s| s.to_str()) {
                if name.starts_with("python") || name == "rt" {
                    continue;
                }
            }
        }

        filtered_args.push(arg);
    }

    Ok(filtered_args)
}

fn missing_riotfile_error() -> PyErr {
    eprintln!(
        "error: riotfile.py not found in the current workspace (searched up to the git repository root)."
    );
    PyErr::new::<PySystemExit, _>(1)
}

fn locate_pytest_plugin_dir(py: Python<'_>) -> Option<PathBuf> {
    let module = py.import(crate::constants::PYTEST_PLUGIN_MODULE).ok()?;
    let file_attr = module.getattr("__file__").ok()?;
    let file_path: PathBuf = file_attr.extract().ok()?;
    let parent = file_path.parent()?;
    let plugin_path = parent.join("rt.py");
    (parent.is_dir() && plugin_path.is_file()).then(|| parent.to_path_buf())
}

/// Entry point used by the Python console-script proxy.
#[pyfunction]
fn run_cli(py: Python<'_>) -> PyResult<()> {
    let cli_args = prepare_cli_args(py)?;
    completion::prepare(py);

    let current_dir = std::env::current_dir().ok();

    if CompleteEnv::with_factory(Cli::command)
        .try_complete(&cli_args, current_dir.as_deref())
        .map_err(|err| {
            eprintln!("error: failed to complete: {err}");
            PyErr::new::<PySystemExit, _>(1)
        })?
    {
        return Ok(());
    }

    let cli = Cli::try_parse_from(cli_args).map_err(|err| {
        let _ = err.print();
        PyErr::new::<PySystemExit, _>(err.exit_code())
    })?;

    if let Commands::Completions { shell } = cli.command {
        commands::completion::run(shell)?;
        return Ok(());
    }

    let riotfile_path = if let Some(path) = &cli.file {
        if !path.is_file() {
            eprintln!(
                "error: specified riotfile {} does not exist.",
                path.display()
            );
            return Err(PyErr::new::<PySystemExit, _>(1));
        }
        path.to_owned()
    } else if let Some(path) = locate_riotfile(None) {
        path
    } else {
        return Err(missing_riotfile_error());
    };

    let Some(riot_root) = locate_riotroot(&riotfile_path, cli.riot_root.as_ref()) else {
        eprintln!("error: could not create riot root directory");
        return Err(PyErr::new::<PySystemExit, _>(1));
    };

    let pytest_plugin_dir = locate_pytest_plugin_dir(py);
    if pytest_plugin_dir.is_none() {
        eprintln!(
            "warning: rt pytest plugin could not be located; pytest integration may be disabled."
        );
    }

    let mut repo_config = RepoConfig::load(&riotfile_path, &riot_root)?;
    repo_config.pytest_plugin_dir = pytest_plugin_dir;

    run_command(py, cli, &repo_config)
}

/// A Python module implemented in Rust.
#[pymodule]
fn riot(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(run_cli, m)?)?;
    m.add_class::<venv::PyVenv>()?;

    Ok(())
}
