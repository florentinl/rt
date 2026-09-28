#![warn(clippy::all)]
#![warn(clippy::pedantic)]
#![warn(clippy::nursery)]
#![allow(clippy::literal_string_with_formatting_args)]

use clap::{CommandFactory, Parser};
use clap_complete::CompleteEnv;
use pyo3::prelude::*;

mod command;
mod commands;
mod completion;
mod config;
mod constants;
mod display;
mod error;
mod progress;
mod suitespec;
mod ui;
mod uv_project;
mod venv;

use crate::{
    config::{RepoConfig, Selector},
    error::{RtError, RtResult},
    venv::Suite,
};
use clap::{Subcommand, ValueHint};
use clap_complete::engine::ArgValueCompleter;
use indexmap::IndexMap;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};

static RUNTIME_PROJECT: OnceLock<PathBuf> = OnceLock::new();

#[pymodule]
fn _rt(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let module_path = PathBuf::from(m.filename()?.to_str()?);
    let module_dir = module_path.parent().ok_or_else(|| {
        pyo3::exceptions::PyRuntimeError::new_err("rt native module has no parent directory")
    })?;
    RUNTIME_PROJECT.get_or_init(|| module_dir.join("_runtime_project"));

    m.add_function(wrap_pyfunction!(cli_main, m)?)?;
    Ok(())
}

#[pyfunction]
fn cli_main(args: Vec<String>) -> u8 {
    let current_dir = std::env::current_dir().ok();
    match CompleteEnv::with_factory(Cli::command).try_complete(args.clone(), current_dir.as_deref())
    {
        Ok(true) => return 0,
        Ok(false) => {}
        Err(err) => err.exit(),
    }

    match try_main(args) {
        Ok(()) => 0,
        Err(err) => {
            err.report();
            err.exit_code()
        }
    }
}

#[derive(Parser)]
#[command(
    name = "rt",
    version,
    about = "CLI proxy exported from the rt native extension."
)]
struct Cli {
    /// Project root containing tests/suitespec.py. Falls back to discovery.
    #[arg(short, long, value_name = "PATH", add = ValueHint::DirPath)]
    pub project_root: Option<PathBuf>,
    /// Cache root. Defaults to <project>/.cache/rt.
    #[arg(long, value_name = "PATH", add = ValueHint::DirPath)]
    pub cache_root: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List normalized suites, test environments, and runs.
    #[command(alias = "ls")]
    List {
        /// Print only unique test-environment hashes.
        #[arg(long = "hash-only")]
        hash_only: bool,
        /// Output the suite-first schema as JSON instead of a hierarchy.
        #[arg(long = "json")]
        json: bool,
        /// Optional suite ID, environment hash, run ID, or name regular expression.
        #[arg(
            value_name = "NAME_PATTERN",
            add = ArgValueCompleter::new(completion::NameCompleter)
        )]
        pattern: Option<String>,
        /// Filter test environments to specific Python versions.
        #[arg(
            short = 'p',
            long = "python",
            value_name = "PYTHON",
            add = ArgValueCompleter::new(completion::PythonCompleter)
        )]
        python: Option<Vec<String>>,
        /// Filter to suites whose patterns include this path and do not exclude it.
        #[arg(short = 't', long = "test", value_name = "PYTEST_TARGET")]
        test: Option<String>,
    },
    /// Show a normalized test environment or run.
    Describe {
        /// Test-environment hash or run ID.
        #[arg(
            value_name = "HASH",
            add = ArgValueCompleter::new(completion::HashCompleter)
        )]
        hash: String,
    },
    /// Build test environments matched by the selector.
    Build {
        /// Force reinstalling cached dependencies before building.
        #[arg(long = "force-reinstall")]
        force_reinstall: bool,
        /// Install the project in non-editable mode (copies built package instead of linking to source).
        #[arg(long = "no-editable")]
        no_editable: bool,
        /// Filter test environments to specific Python versions.
        #[arg(
            short = 'p',
            long = "python",
            value_name = "PYTHON",
            add = ArgValueCompleter::new(completion::PythonCompleter)
        )]
        python: Option<Vec<String>>,
        /// Suite ID, environment hash, run ID, or environment-name regex.
        #[arg(
            value_name = "PATTERN",
            required = true,
            add = ArgValueCompleter::new(completion::SelectorCompleter)
        )]
        pattern: Option<String>,
        /// Filter to suites whose patterns include this path and do not exclude it.
        #[arg(short = 't', long = "test", value_name = "PYTEST_TARGET")]
        test: Option<String>,
    },
    /// Build and execute test runs matched by the selector.
    Run {
        /// Force reinstalling cached dependencies before running.
        #[arg(long = "force-reinstall")]
        force_reinstall: bool,
        /// Install the project in non-editable mode (copies built package instead of linking to source).
        #[arg(long = "no-editable")]
        no_editable: bool,
        /// Run in parallel (optionally specify worker count).
        #[arg(
            long = "parallel",
            value_name = "N",
            num_args = 0..=1,
            default_missing_value = "10"
        )]
        parallel: Option<usize>,
        /// Override the suitespec run command template.
        #[arg(long = "command", value_name = "COMMAND")]
        command_override: Option<String>,
        /// Filter test environments to specific Python versions.
        #[arg(
            short = 'p',
            long = "python",
            value_name = "PYTHON",
            add = ArgValueCompleter::new(completion::PythonCompleter)
        )]
        python: Option<Vec<String>>,
        /// Suite ID, environment hash, run ID, or environment-name regex.
        #[arg(
            value_name = "PATTERN",
            required = true,
            add = ArgValueCompleter::new(completion::SelectorCompleter)
        )]
        pattern: String,
        /// Filter to suites whose patterns include this path and do not exclude it.
        #[arg(short = 't', long = "test", value_name = "PYTEST_TARGET")]
        test: Option<String>,
        /// Arguments substituted for `{cmdargs}` in the suitespec command.
        #[arg(value_name = "CMDARGS", trailing_var_arg = true)]
        cmdargs: Vec<String>,
    },
    /// Build a test run profile and start a shell with it active.
    Shell {
        /// Test-environment hash or run ID.
        #[arg(
            value_name = "HASH",
            add = ArgValueCompleter::new(completion::HashCompleter)
        )]
        hash: String,
        /// Force reinstalling cached dependencies before opening the shell.
        #[arg(long = "force-reinstall")]
        force_reinstall: bool,
    },
    /// Build a test run profile and print its activation script path.
    Activate {
        /// Test-environment hash or run ID.
        #[arg(
            value_name = "HASH",
            add = ArgValueCompleter::new(completion::HashCompleter)
        )]
        hash: String,
        /// Force reinstalling cached dependencies before preparing the environment.
        #[arg(long = "force-reinstall")]
        force_reinstall: bool,
    },
    /// Build the test profile and link it as .venv in the project root.
    Switch {
        /// Test-environment hash or run ID.
        #[arg(
            value_name = "HASH",
            add = ArgValueCompleter::new(completion::HashCompleter)
        )]
        hash: String,
        /// Force reinstalling cached dependencies before preparing the environment.
        #[arg(long = "force-reinstall")]
        force_reinstall: bool,
    },
    /// Remove the rt cache for this project.
    Clean,
    #[command(name = "_is_venv_ready", hide = true)]
    IsVenvReady {
        #[arg(value_name = "HASH")]
        hash: String,
        #[arg(long, value_name = "FINGERPRINT")]
        profile_fingerprint: Option<String>,
    },
}

/// Dispatch the selected CLI command.
///
/// # Errors
///
/// Returns an error if command execution fails.
fn run_command(suites: IndexMap<String, Suite>, cli: Cli, repo: &RepoConfig) -> RtResult<()> {
    match cli.command {
        Commands::List {
            hash_only,
            json,
            pattern,
            python,
            test,
        } => {
            if hash_only && json {
                return Err(RtError::with_code(
                    2,
                    "error: --hash-only and --json cannot be used together.",
                ));
            }
            let selector = Selector::Generic {
                python,
                pattern,
                test,
            };
            commands::list::run(suites, repo, selector, hash_only, json)
        }
        Commands::Describe { hash } => commands::describe::run(&suites, repo, hash),
        Commands::Build {
            force_reinstall,
            no_editable,
            pattern,
            python,
            test,
        } => commands::build::run(
            &suites,
            repo,
            Selector::Generic {
                python,
                pattern,
                test,
            },
            force_reinstall,
            no_editable,
        ),
        Commands::Run {
            force_reinstall,
            no_editable,
            parallel,
            command_override,
            python,
            pattern,
            test,
            cmdargs,
        } => commands::run::run(
            &suites,
            repo,
            Selector::Generic {
                python,
                pattern: Some(pattern),
                test,
            },
            &commands::run::Options {
                force_reinstall,
                no_editable,
                parallel,
                command_override,
                cmdargs,
            },
        ),
        Commands::Shell {
            hash,
            force_reinstall,
        } => commands::shell::run(&suites, repo, &hash, force_reinstall),
        Commands::Activate {
            hash,
            force_reinstall,
        } => commands::activate::run(&suites, repo, &hash, force_reinstall),
        Commands::Switch {
            hash,
            force_reinstall,
        } => commands::switch::run(&suites, repo, &hash, force_reinstall),
        Commands::Clean => commands::clean::run(&repo.cache_root),
        Commands::IsVenvReady {
            hash,
            profile_fingerprint,
        } => commands::is_venv_ready::run(&repo.cache_root, &hash, profile_fingerprint.as_deref()),
    }
}

fn locate_project_root(project_root: Option<&PathBuf>) -> RtResult<PathBuf> {
    if let Some(path) = project_root {
        if !path.join("tests/suitespec.py").is_file() {
            return Err(RtError::message(format!(
                "error: specified project root {} does not contain tests/suitespec.py.",
                path.display()
            )));
        }
        return fs::canonicalize(path).map_err(|err| {
            RtError::message(format!(
                "error: failed to resolve project root {}: {err}",
                path.display()
            ))
        });
    }

    let mut dir = std::env::current_dir().map_err(|err| {
        RtError::message(format!(
            "error: failed to determine current directory: {err}"
        ))
    })?;

    loop {
        if dir.join("tests/suitespec.py").is_file() {
            return Ok(dir);
        }

        if dir.join(".git").is_dir() {
            break;
        }

        if !dir.pop() {
            break;
        }
    }

    Err(RtError::message(
        "error: tests/suitespec.py not found in the current workspace (searched up to the git repository root).",
    ))
}

fn locate_cache_root(project_root: &Path, cache_root: Option<&PathBuf>) -> RtResult<PathBuf> {
    let path = cache_root.map_or_else(
        || project_root.join(".cache/rt"),
        |path| {
            if path.is_absolute() {
                path.clone()
            } else {
                std::env::current_dir().map_or_else(|_| path.clone(), |current| current.join(path))
            }
        },
    );
    path.parent()
        .is_some_and(Path::exists)
        .then_some(path)
        .ok_or_else(|| RtError::message("error: cache root parent does not exist"))
}

fn runtime_project_path() -> RtResult<PathBuf> {
    RUNTIME_PROJECT
        .get()
        .cloned()
        .ok_or_else(|| RtError::message("error: could not locate the bundled rt runtime project"))
}

fn try_main(args: Vec<String>) -> RtResult<()> {
    let cli = Cli::parse_from(args);

    let project_root = locate_project_root(cli.project_root.as_ref())?;
    let cache_root = locate_cache_root(&project_root, cli.cache_root.as_ref())?;

    // Cache maintenance and readiness checks only need the cache root. In
    // particular, they must remain usable when the project's Python
    // configuration is temporarily broken.
    match &cli.command {
        Commands::Clean => return commands::clean::run(&cache_root),
        Commands::IsVenvReady {
            hash,
            profile_fingerprint,
        } => {
            return commands::is_venv_ready::run(&cache_root, hash, profile_fingerprint.as_deref());
        }
        _ => {}
    }

    let suites = venv::load_context(&project_root, Some(&cache_root))?;

    let repo_config = RepoConfig {
        project_root,
        cache_root,
    };

    run_command(suites, cli, &repo_config)
}
