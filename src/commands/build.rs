use std::{
    fs,
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, OnceLock},
};

use indexmap::IndexMap;
use rayon::current_num_threads;
use serde::Serialize;

use crate::{
    command::ManagedCommand,
    config::{RepoConfig, Selector},
    constants::{RT_VERSION, RUNTIME_CONFIG},
    error::{RtError, RtResult},
    progress::{
        MultiplexedProgressLogger, PlainProgressLogger, ProgressLogger, StepContext, StepId, Task,
        TaskRunner, summarize_errors,
    },
    uv_project::{ensure_uv_project, has_pkg},
    venv::{
        Suite, TestEnvironment, TestRun, environment_path, profile_fingerprint, profile_path,
        select_test_environments,
    },
};

pub fn run(
    suites: &IndexMap<String, Suite>,
    repo: &RepoConfig,
    selector: Selector,
    force_reinstall: bool,
    no_editable: bool,
) -> RtResult<()> {
    let selected = select_test_environments(suites, selector)?;
    build_selected_environments(repo, &selected, force_reinstall, no_editable)
}

pub fn build_selected_environments(
    repo: &RepoConfig,
    selected: &[TestEnvironment],
    force_reinstall: bool,
    no_editable: bool,
) -> RtResult<()> {
    fs::create_dir_all(&repo.cache_root).map_err(|error| {
        RtError::message(format!("error: could not create cache root: {error}"))
    })?;
    let sink: Arc<dyn ProgressLogger> = if io::stderr().is_terminal() {
        MultiplexedProgressLogger::new().map_or_else(
            |_| Arc::new(PlainProgressLogger::default()) as Arc<_>,
            |logger| Arc::new(logger) as Arc<_>,
        )
    } else {
        Arc::new(PlainProgressLogger::default())
    };
    let state = Arc::new(BuildSharedState::new(
        force_reinstall,
        no_editable,
        repo.cache_root.clone(),
        repo.project_root.clone(),
        crate::runtime_project_path()?,
    ));
    let runner = TaskRunner::new(Arc::clone(&sink)).with_parallelism(Some(current_num_threads()));
    let mut unique = IndexMap::new();
    for environment in selected {
        unique
            .entry(environment.hash.clone())
            .or_insert(environment);
    }
    let tasks: Vec<_> = unique
        .into_values()
        .map(|environment| {
            let state = Arc::clone(&state);
            let label = format!("sync test environment {}", environment.hash);
            Task::new(StepId::new(&label), &label, move |context| {
                state.ensure_environment(environment, &context)
            })
        })
        .collect();
    let errors = runner.run(tasks).map_err(|error| {
        RtError::message(format!(
            "error: could not configure build parallelism ({error})"
        ))
    })?;
    if summarize_errors(&errors, "build") {
        return Err(RtError::silent(1));
    }
    Ok(())
}

struct BuildSharedState {
    force_reinstall: bool,
    no_editable: bool,
    cache_root: PathBuf,
    project_root: PathBuf,
    runtime_project: PathBuf,
}

impl BuildSharedState {
    const fn new(
        force_reinstall: bool,
        no_editable: bool,
        cache_root: PathBuf,
        project_root: PathBuf,
        runtime_project: PathBuf,
    ) -> Self {
        Self {
            force_reinstall,
            no_editable,
            cache_root,
            project_root,
            runtime_project,
        }
    }

    fn ensure_environment(
        &self,
        environment: &TestEnvironment,
        context: &StepContext,
    ) -> RtResult<()> {
        let physical = environment_path(&self.cache_root, &environment.hash);
        if self.force_reinstall && physical.exists() {
            fs::remove_dir_all(&physical)?;
        }
        let uv_project = ensure_uv_project(
            &self.cache_root,
            &self.project_root,
            &self.runtime_project,
            environment,
            self.no_editable,
        )?;
        let mut command =
            ManagedCommand::new_uv("sync", Arc::clone(&context.sink), context.step_id.clone());
        if has_pkg(environment, "pylibmc")
            && let Some(prefix) = libmemcached_prefix()
        {
            command = command.env("LIBMEMCACHED", prefix);
        }
        command = command
            .env("DD_FAST_BUILD", "1")
            .env("UV_PROJECT_ENVIRONMENT", &physical)
            .arg("--project")
            .arg(&uv_project)
            .arg("--python")
            .arg(&environment.python);
        if self.force_reinstall {
            command = command.arg("--reinstall");
        }
        if self.no_editable {
            command = command.arg("--no-editable");
        } else {
            command = command.args(["--config-setting", "editable_mode=compat"]);
        }
        let status = command.status()?;
        if !status.success() {
            return Err(RtError::message(format!(
                "error: uv sync failed with status {status}"
            )));
        }
        for run in &environment.runs {
            self.write_profile(environment, run, &physical)?;
        }
        Ok(())
    }

    fn write_profile(
        &self,
        environment: &TestEnvironment,
        run: &TestRun,
        physical: &Path,
    ) -> RtResult<()> {
        let profile = profile_path(&self.cache_root, &environment.hash, run.index);
        let bin = profile.join("bin");
        fs::create_dir_all(&bin)?;
        let mut values = environment.suite_env.clone();
        values.extend(run.env.clone());
        values.insert(
            "TEST_SUITE".to_owned(),
            environment.integration_name.clone(),
        );
        let project_root = fs::canonicalize(&self.project_root)
            .unwrap_or_else(|_| self.project_root.clone())
            .to_string_lossy()
            .into_owned();
        let manifest = RuntimeContextManifest {
            rt_version: RT_VERSION,
            profile_fingerprint: profile_fingerprint(environment, run, self.no_editable),
            project_root: &project_root,
            no_editable: self.no_editable,
            environment: values,
            services: &environment.services,
            command: &run.command,
        };
        atomic_write_json(&profile.join(RUNTIME_CONFIG), &manifest)?;
        let manifest_path = fs::canonicalize(profile.join(RUNTIME_CONFIG))
            .unwrap_or_else(|_| profile.join(RUNTIME_CONFIG));
        let physical_python = physical.join(if cfg!(windows) {
            "Scripts/python.exe"
        } else {
            "bin/python"
        });
        #[cfg(unix)]
        {
            write_executable(
                &bin.join("python"),
                &format!(
                    "#!/bin/sh\nexport RT_CONTEXT_PATH={}\nexec {} \"$@\"\n",
                    shell_quote(&manifest_path),
                    shell_quote(&physical_python)
                ),
            )?;
            write_executable(
                &bin.join("activate"),
                &format!(
                    "export RT_CONTEXT_PATH={}\n. {}\n",
                    shell_quote(&manifest_path),
                    shell_quote(&physical.join("bin/activate"))
                ),
            )?;
        }
        Ok(())
    }
}

#[derive(Serialize)]
struct RuntimeContextManifest<'a> {
    rt_version: &'static str,
    profile_fingerprint: String,
    project_root: &'a str,
    no_editable: bool,
    environment: IndexMap<String, String>,
    services: &'a [String],
    command: &'a str,
}

fn atomic_write_json(path: &Path, value: &impl Serialize) -> RtResult<()> {
    let mut contents = serde_json::to_string_pretty(value).map_err(|error| {
        RtError::message(format!(
            "error: failed to serialize runtime profile: {error}"
        ))
    })?;
    contents.push('\n');
    let parent = path
        .parent()
        .ok_or_else(|| RtError::message("error: runtime profile has no parent"))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(contents.as_bytes())?;
    temporary.flush()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(unix)]
fn write_executable(path: &Path, contents: &str) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::write(path, contents)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

fn libmemcached_prefix() -> Option<String> {
    static PREFIX: OnceLock<Option<String>> = OnceLock::new();
    PREFIX
        .get_or_init(|| {
            if let Some(prefix) = std::env::var_os("LIBMEMCACHED")
                && !prefix.is_empty()
            {
                return Some(prefix.to_string_lossy().into_owned());
            }
            Command::new("brew")
                .args(["--prefix", "libmemcached"])
                .stderr(Stdio::null())
                .output()
                .ok()
                .filter(|output| output.status.success())
                .and_then(|output| String::from_utf8(output.stdout).ok())
                .map(|prefix| prefix.trim().to_owned())
                .filter(|prefix| !prefix.is_empty())
                .map(PathBuf::from)
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
                .map(|path| path.to_string_lossy().into_owned())
        })
        .clone()
}
