use std::{
    collections::HashMap,
    fmt::Write as FmtWrite,
    fs,
    io::{self, IsTerminal},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, OnceLock},
};

use crate::{
    command::ManagedCommand,
    config::Selector,
    error::{RtError, RtResult},
    progress::{
        MultiplexedProgressLogger, PlainProgressLogger, ProgressLogger, StepContext, StepId,
        StepOutcome, Task, TaskRunner, summarize_errors,
    },
    uv_project::{ensure_uv_project, has_pkg},
    venv::{ExecutionContext, RiotVenv, venv_path},
};
use indexmap::IndexMap;

use rayon::current_num_threads;

use crate::{
    config::RepoConfig,
    constants::{DONE_MARKER, RT_VERSION},
    venv::select_execution_contexts,
};

/// Build the virtual environment for the provided execution context.
///
/// # Errors
///
/// Returns an error when context selection fails or any build step fails.
pub fn run(
    venvs: IndexMap<String, RiotVenv>,
    repo: &RepoConfig,
    selector: Selector,
    force_reinstall: bool,
    no_editable: bool,
) -> RtResult<()> {
    let selected = select_execution_contexts(venvs, selector)?;
    build_selected_contexts(repo, &selected, force_reinstall, no_editable)?;
    Ok(())
}

#[must_use]
pub fn collect_context_indices(selected: &[RiotVenv]) -> Vec<(usize, usize)> {
    selected
        .iter()
        .enumerate()
        .flat_map(|(venv_idx, selected_venv)| {
            selected_venv
                .execution_contexts
                .iter()
                .enumerate()
                .map(move |(ctx_idx, _)| (venv_idx, ctx_idx))
        })
        .collect()
}

/// Build every selected execution context.
///
/// # Errors
///
/// Returns an error when the riot root cannot be prepared or any task fails.
pub fn build_selected_contexts(
    repo: &RepoConfig,
    selected: &[RiotVenv],
    force_reinstall: bool,
    no_editable: bool,
) -> RtResult<()> {
    if let Err(e) = fs::DirBuilder::new()
        .recursive(true)
        .create(&repo.riot_root)
    {
        return Err(RtError::message(format!(
            "error: could not create riot root: {e}"
        )));
    }
    let sink: Arc<dyn ProgressLogger> = if io::stderr().is_terminal() {
        match MultiplexedProgressLogger::new() {
            Ok(logger) => Arc::new(logger),
            Err(_) => Arc::new(PlainProgressLogger::default()),
        }
    } else {
        Arc::new(PlainProgressLogger::default())
    };
    let shared = Arc::new(BuildSharedState::new(
        force_reinstall,
        no_editable,
        Arc::clone(&repo.build_env),
        Arc::clone(&repo.run_env),
        repo.riot_root.clone(),
        repo.riotfile_path
            .parent()
            .ok_or_else(|| RtError::message("error: could not find riotfile parent"))?
            .to_path_buf(),
    ));
    let runner = TaskRunner::new(Arc::clone(&sink)).with_parallelism(Some(current_num_threads()));

    let context_indices = collect_context_indices(selected);

    let exc_ctx_tasks: Vec<_> = context_indices
        .iter()
        .map(|&(venv_i, exc_i)| {
            let state = Arc::clone(&shared);
            let venv = &selected[venv_i];
            let exc_ctx = &venv.execution_contexts[exc_i];
            let step_id = format!("sync execution context {}", exc_ctx.hash);
            Task::new(StepId::new(&step_id), &step_id, move |ctx| {
                state.ensure_execution_ctx(venv, exc_ctx, &ctx)
            })
        })
        .collect();

    let exc_errors = runner.run(exc_ctx_tasks).map_err(|err| {
        RtError::message(format!(
            "error: could not configure build parallelism ({err})"
        ))
    })?;

    if summarize_errors(&exc_errors, "build") {
        return Err(RtError::silent(1));
    }

    Ok(())
}

type DynResult<T> = RtResult<T>;

pub struct BuildSharedState {
    force_reinstall: bool,
    no_editable: bool,
    build_env: Arc<HashMap<String, String>>,
    run_env: Arc<HashMap<String, String>>,
    riot_root: PathBuf,
    project_root: PathBuf,
}

impl BuildSharedState {
    #[must_use]
    pub const fn new(
        force_reinstall: bool,
        no_editable: bool,
        build_env: Arc<HashMap<String, String>>,
        run_env: Arc<HashMap<String, String>>,
        riot_root: PathBuf,
        project_root: PathBuf,
    ) -> Self {
        Self {
            force_reinstall,
            no_editable,
            build_env,
            run_env,
            riot_root,
            project_root,
        }
    }

    fn ensure_execution_ctx(
        &self,
        venv: &RiotVenv,
        exc: &ExecutionContext,
        ctx: &StepContext,
    ) -> DynResult<StepOutcome> {
        let exc_venv_path = venv_path(&self.riot_root, &exc.hash);
        let marker_path = exc_venv_path.join(DONE_MARKER);

        if self.force_reinstall && exc_venv_path.exists() {
            fs::remove_dir_all(&exc_venv_path)?;
        }

        let uv_project = ensure_uv_project(&self.riot_root, &self.project_root, venv, exc)?;

        let mut cmd = ManagedCommand::new_uv("sync", Arc::clone(&ctx.sink), ctx.step_id.clone());

        // pylibmc has no wheels: it compiles against libmemcached, which the CI
        // image installs system-wide (libmemcached-dev). On developer machines
        // the headers usually live under a Homebrew prefix, so point the build
        // at it. Applied before build_env so rt.toml can still override it.
        if has_pkg(venv, "pylibmc")
            && let Some(prefix) = libmemcached_prefix()
        {
            cmd = cmd.env("LIBMEMCACHED", prefix);
        }

        cmd = cmd
            .envs(self.build_env.as_ref())
            .env("DD_FAST_BUILD", "1")
            .env("UV_PROJECT_ENVIRONMENT", &exc_venv_path)
            .arg("--project")
            .arg(&uv_project)
            .arg("--python")
            .arg(&venv.python);

        if self.force_reinstall {
            cmd = cmd.arg("--reinstall");
        }

        if self.no_editable {
            cmd = cmd.arg("--no-editable");
        } else {
            cmd = cmd.args(["--config-setting", "editable_mode=compat"]);
        }

        let status = cmd.status()?;

        if !status.success() {
            return Err(RtError::message(format!(
                "error: uv sync failed with status {status}"
            )));
        }

        let site_packages_path =
            exc_venv_path.join(format!("lib/python{}/site-packages", &venv.python));

        self.configure_site_packages(venv, exc, &site_packages_path)?;

        write_marker(&marker_path)?;

        Ok(StepOutcome::Done)
    }

    fn configure_site_packages(
        &self,
        venv: &RiotVenv,
        exc: &ExecutionContext,
        site_packages_path: &Path,
    ) -> RtResult<()> {
        fs::create_dir_all(site_packages_path)?;

        let current_dir =
            fs::canonicalize(&self.project_root).unwrap_or_else(|_| self.project_root.clone());

        {
            fs::write(
                site_packages_path.join("pytest_rt.py"),
                include_str!("../../pytest_rt/pytest_rt.py"),
            )?;
            fs::write(
                site_packages_path.join("enable_pytest_rt.py"),
                include_str!("../../pytest_rt/enable_pytest_rt.py"),
            )?;
        }

        // envier is a ddtrace runtime dependency, so it lands in the venv
        // through the local project dependency rather than through a riotfile
        // package. A context that skips the project only has it when the
        // riotfile asks for it explicitly.
        if !exc.skip_dev_install || has_pkg(venv, "envier") {
            write_envier_stubs(site_packages_path)?;
        }

        self.write_sitecustomize(
            site_packages_path,
            &current_dir,
            exc,
            &venv.services,
            &venv.suite_env,
        )?;

        Ok(())
    }

    fn write_sitecustomize(
        &self,
        site_packages_path: &Path,
        current_dir: &Path,
        exc: &ExecutionContext,
        services: &[String],
        suite_env: &IndexMap<String, String>,
    ) -> RtResult<()> {
        let sitecustomize_path = site_packages_path.join("sitecustomize.py");
        let mut sc = String::new();
        sc.push_str("import os\n");

        if self.no_editable {
            // In non-editable mode the installed package must take precedence
            // over the local source tree.  Python puts '' (CWD) at sys.path[0]
            // which would shadow the installed copy.  We remove both '' and the
            // absolute project root, then re-append the project root at the end
            // so test helpers remain importable but lose priority.
            let project_root_str = current_dir.to_string_lossy();
            writeln!(sc, "\nimport sys")?;
            writeln!(
                sc,
                "sys.path[:] = [p for p in sys.path if p not in ('', r\"{project_root_str}\")]"
            )?;
            writeln!(
                sc,
                "sys.path.append(r\"{project_root_str}\")  # test helpers still importable, but last"
            )?;
        }

        // environment variables from tests/suitespec.yml, exported by
        // scripts/run-tests before it hands over to riot. riotfile.py env vars
        // are applied after these so they keep winning, exactly like riot
        // overriding the inherited process environment.
        if !suite_env.is_empty() {
            sc.push_str("\n# Environment variables from tests/suitespec.yml\n");
            for (key, val) in suite_env {
                writeln!(sc, "os.environ[r\"{key}\"] = r\"{val}\"")?;
            }
        }

        // environment variables from riotfile.py
        if !exc.env.is_empty() {
            sc.push_str("\n# Environment variables from riotfile.py\n");
            for (key, val) in &exc.env {
                writeln!(sc, "os.environ[r\"{key}\"] = r\"{val}\"")?;
            }
        }

        // environment variables from rt.toml
        if !self.run_env.is_empty() {
            sc.push_str("\n# Environment variables from rt.toml\n");
            for (key, val) in self.run_env.iter() {
                writeln!(sc, "os.environ[r\"{key}\"] = r\"{val}\"")?;
            }
        }

        writeln!(sc)?;
        writeln!(
            sc,
            "# Pytest hacks for dd-trace-py using a pytest plugin that starts/stops suitespec services, trims python versions (ex: [3.13]) from test names and resets datadog service name based on the original riotfile command"
        )?;
        writeln!(sc, "import enable_pytest_rt")?;

        let services_csv = services.join(",");
        let project_root = current_dir.to_string_lossy();

        writeln!(
            sc,
            "os.environ[\"PYTHONPATH\"] = r\"{project_root}\" # Functionally useless but some tests assume that this env var is not empty..."
        )?;
        if services.iter().any(|svc| svc == "testagent") {
            writeln!(
                sc,
                "os.environ[\"DD_TRACE_AGENT_URL\"] = \"http://localhost:9126\" # default testagent url; pytest_rt refines it from the published compose port"
            )?;
        }

        writeln!(
            sc,
            "os.environ[\"RIOT_SUITESPEC_SERVICES\"] = r\"{services_csv}\" # store services to start for pytest_rt plugin"
        )?;
        writeln!(
            sc,
            "os.environ[\"RIOT_PROJECT_ROOT\"] = r\"{project_root}\" # store riot project root for pytest_rt plugin"
        )?;
        if let Some(command) = exc.command.as_ref() {
            writeln!(
                sc,
                "os.environ[\"RIOT_ORIGINAL_COMMAND\"] = r\"{command}\" # store riotfile.py command for pytest_rt plugin"
            )?;
        }

        fs::write(sitecustomize_path, sc)?;
        Ok(())
    }
}

/// Vendored `types-envier` stub package, embedded at compile time.
///
/// `envier` ships no `py.typed` and no inline annotations, so every setting
/// declared through it resolves to an untyped attribute in the editor. The
/// stubs are a PEP 561 stub-only package: a plain `envier-stubs` directory next
/// to the real package is all a type checker needs, so there is nothing to
/// install and nothing to resolve over the network.
///
/// See `vendor/types-envier/VENDORING.md` for provenance and update steps.
const ENVIER_STUBS: &[(&str, &str)] = &[
    (
        "__init__.pyi",
        include_str!("../../vendor/types-envier/envier-stubs/__init__.pyi"),
    ),
    (
        "env.pyi",
        include_str!("../../vendor/types-envier/envier-stubs/env.pyi"),
    ),
    (
        "mypy.pyi",
        include_str!("../../vendor/types-envier/envier-stubs/mypy.pyi"),
    ),
    (
        "validators.pyi",
        include_str!("../../vendor/types-envier/envier-stubs/validators.pyi"),
    ),
];

fn write_envier_stubs(site_packages_path: &Path) -> RtResult<()> {
    let stubs_path = site_packages_path.join("envier-stubs");
    fs::create_dir_all(&stubs_path)?;

    for (name, contents) in ENVIER_STUBS {
        fs::write(stubs_path.join(name), contents)?;
    }

    Ok(())
}

fn write_marker(marker_path: &Path) -> io::Result<()> {
    fs::write(marker_path, format!("{RT_VERSION}\n"))
}

/// Locate a libmemcached installation for building `pylibmc`.
///
/// Resolution order: an explicit `LIBMEMCACHED` in the environment, then
/// `brew --prefix libmemcached`, then the well-known Homebrew prefixes.
fn libmemcached_prefix() -> Option<String> {
    static PREFIX: OnceLock<Option<String>> = OnceLock::new();

    PREFIX
        .get_or_init(|| {
            if let Some(prefix) = std::env::var_os("LIBMEMCACHED")
                && !prefix.is_empty()
            {
                return Some(prefix.to_string_lossy().into_owned());
            }

            brew_prefix("libmemcached")
                .into_iter()
                .chain(
                    [
                        "/opt/homebrew/opt/libmemcached",
                        "/usr/local/opt/libmemcached",
                        "/opt/homebrew",
                        "/usr/local",
                    ]
                    .into_iter()
                    .map(PathBuf::from),
                )
                .find(|candidate| candidate.join("include/libmemcached/memcached.h").is_file())
                .map(|prefix| prefix.to_string_lossy().into_owned())
        })
        .clone()
}

fn brew_prefix(formula: &str) -> Option<PathBuf> {
    let output = Command::new("brew")
        .args(["--prefix", formula])
        .stderr(Stdio::null())
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let prefix = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (!prefix.is_empty()).then(|| PathBuf::from(prefix))
}

#[cfg(test)]
mod tests {
    use super::{ENVIER_STUBS, write_envier_stubs, write_marker};
    use crate::constants::{DONE_MARKER, RT_VERSION};
    use std::fs;

    #[test]
    fn envier_stubs_are_written_as_a_pep561_stub_package() {
        let dir = tempfile::tempdir().unwrap();

        write_envier_stubs(dir.path()).unwrap();

        let stubs = dir.path().join("envier-stubs");
        assert!(stubs.join("__init__.pyi").is_file());
        assert!(stubs.join("env.pyi").is_file());
        assert!(stubs.join("mypy.pyi").is_file());
        assert!(stubs.join("validators.pyi").is_file());

        // The vendored copies must stay verbatim so upstream stubtest results
        // remain comparable.
        let env_stub = fs::read_to_string(stubs.join("env.pyi")).unwrap();
        assert!(env_stub.contains("class EnvVariable(Generic[T]):"));
    }

    #[test]
    fn every_vendored_stub_is_embedded_and_non_empty() {
        assert_eq!(ENVIER_STUBS.len(), 4);
        for (name, contents) in ENVIER_STUBS {
            assert!(
                std::path::Path::new(name)
                    .extension()
                    .is_some_and(|ext| ext == "pyi"),
                "unexpected stub file: {name}"
            );
            assert!(!contents.trim().is_empty(), "empty stub file: {name}");
        }
    }

    #[test]
    fn marker_records_the_rt_version() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join(DONE_MARKER);

        write_marker(&marker).unwrap();

        assert_eq!(fs::read_to_string(&marker).unwrap().trim(), RT_VERSION);
    }
}
