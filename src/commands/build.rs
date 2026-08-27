use std::{
    fs,
    io::{self, IsTerminal, Write as _},
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
use serde::Serialize;

use rayon::current_num_threads;

use crate::{
    config::RepoConfig,
    constants::{RT_VERSION, RUNTIME_CONFIG},
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
    build_selected_contexts(repo, &selected, force_reinstall, no_editable)
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
    fs::create_dir_all(&repo.riot_root)
        .map_err(|err| RtError::message(format!("error: could not create riot root: {err}")))?;
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
        repo.riot_root.clone(),
        repo.riotfile_path
            .parent()
            .ok_or_else(|| RtError::message("error: could not find riotfile parent"))?
            .to_path_buf(),
        crate::runtime_project_path()?,
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

struct BuildSharedState {
    force_reinstall: bool,
    no_editable: bool,
    riot_root: PathBuf,
    project_root: PathBuf,
    runtime_project: PathBuf,
}

impl BuildSharedState {
    #[must_use]
    const fn new(
        force_reinstall: bool,
        no_editable: bool,
        riot_root: PathBuf,
        project_root: PathBuf,
        runtime_project: PathBuf,
    ) -> Self {
        Self {
            force_reinstall,
            no_editable,
            riot_root,
            project_root,
            runtime_project,
        }
    }

    fn ensure_execution_ctx(
        &self,
        venv: &RiotVenv,
        exc: &ExecutionContext,
        ctx: &StepContext,
    ) -> RtResult<StepOutcome> {
        let exc_venv_path = venv_path(&self.riot_root, &exc.hash);
        if self.force_reinstall && exc_venv_path.exists() {
            fs::remove_dir_all(&exc_venv_path)?;
        }

        let uv_project = ensure_uv_project(
            &self.riot_root,
            &self.project_root,
            &self.runtime_project,
            venv,
            exc,
        )?;

        let mut cmd = ManagedCommand::new_uv("sync", Arc::clone(&ctx.sink), ctx.step_id.clone());

        // pylibmc has no wheels: it compiles against libmemcached, which the CI
        // image installs system-wide (libmemcached-dev). On developer machines
        // the headers usually live under a Homebrew prefix, so point the build
        // at it.
        if has_pkg(venv, "pylibmc")
            && let Some(prefix) = libmemcached_prefix()
        {
            cmd = cmd.env("LIBMEMCACHED", prefix);
        }

        cmd = cmd
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

        match fs::remove_file(exc_venv_path.join(RUNTIME_CONFIG)) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }

        let status = cmd.status()?;

        if !status.success() {
            return Err(RtError::message(format!(
                "error: uv sync failed with status {status}"
            )));
        }

        self.write_runtime_context(venv, exc, &exc_venv_path)?;

        Ok(StepOutcome::Done)
    }

    fn write_runtime_context(
        &self,
        venv: &RiotVenv,
        exc: &ExecutionContext,
        environment_path: &Path,
    ) -> RtResult<()> {
        let mut environment = venv.suite_env.clone();

        // Values from riotfile.py override inherited suitespec values.
        environment.extend(exc.env.clone());

        let project_root = fs::canonicalize(&self.project_root)
            .unwrap_or_else(|_| self.project_root.clone())
            .to_string_lossy()
            .into_owned();
        let manifest = RuntimeContextManifest {
            rt_version: RT_VERSION,
            project_root: &project_root,
            no_editable: self.no_editable,
            environment,
            services: &venv.services,
            command: exc.command.as_deref(),
        };
        let mut manifest = serde_json::to_string_pretty(&manifest).map_err(|err| {
            RtError::message(format!("error: failed to serialize runtime context: {err}"))
        })?;
        manifest.push('\n');

        let mut temporary = tempfile::NamedTempFile::new_in(environment_path)?;
        temporary.write_all(manifest.as_bytes())?;
        temporary.flush()?;
        temporary
            .persist(environment_path.join(RUNTIME_CONFIG))
            .map_err(|err| err.error)?;

        Ok(())
    }
}

#[derive(Serialize)]
struct RuntimeContextManifest<'a> {
    rt_version: &'static str,
    project_root: &'a str,
    no_editable: bool,
    environment: IndexMap<String, String>,
    services: &'a [String],
    command: Option<&'a str>,
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
    use super::BuildSharedState;
    use crate::constants::{RT_VERSION, RUNTIME_CONFIG};
    use crate::venv::{ExecutionContext, RiotVenv};
    use indexmap::IndexMap;
    use std::fs;

    #[test]
    fn runtime_context_is_an_atomic_json_completion_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let project_root = dir.path().join("project");
        let environment_path = dir.path().join("venv_context");
        fs::create_dir_all(&project_root).unwrap();
        fs::create_dir_all(&environment_path).unwrap();
        let state = BuildSharedState::new(
            false,
            true,
            dir.path().join(".riot"),
            project_root.clone(),
            dir.path().join("rt-runtime"),
        );
        let mut venv = RiotVenv::empty_for_tests();
        venv.services = vec!["postgres".to_owned(), "testagent".to_owned()];
        venv.suite_env
            .insert("SHARED".to_owned(), "suite".to_owned());
        venv.suite_env
            .insert("SUITE_ONLY".to_owned(), "line\nvalue".to_owned());
        let exc = ExecutionContext {
            command: Some("pytest tests/example.py".to_owned()),
            pytest_targets: Vec::new(),
            env: IndexMap::from([
                ("SHARED".to_owned(), "riotfile".to_owned()),
                ("RIOT_ONLY".to_owned(), "quoted \"value\"".to_owned()),
            ]),
            create: true,
            skip_dev_install: true,
            hash: "context".to_owned(),
        };

        state
            .write_runtime_context(&venv, &exc, &environment_path)
            .unwrap();

        let manifest: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(environment_path.join(RUNTIME_CONFIG)).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["rt_version"], RT_VERSION);
        assert_eq!(
            manifest["project_root"].as_str(),
            project_root.canonicalize().unwrap().to_str()
        );
        assert_eq!(manifest["no_editable"].as_bool(), Some(true));
        assert_eq!(manifest["environment"]["SHARED"], "riotfile");
        assert_eq!(manifest["environment"]["SUITE_ONLY"], "line\nvalue");
        assert_eq!(manifest["environment"]["RIOT_ONLY"], "quoted \"value\"");
        assert_eq!(manifest["services"][0], "postgres");
        assert_eq!(manifest["services"][1], "testagent");
        assert_eq!(manifest["command"], "pytest tests/example.py");
        assert_eq!(fs::read_dir(&environment_path).unwrap().count(), 1);
    }
}
