use std::{
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
use serde::Serialize;

use rayon::current_num_threads;

use crate::{
    config::RepoConfig,
    constants::{DONE_MARKER, RT_VERSION},
    venv::select_execution_contexts,
};

const CONTEXT_MANIFEST: &str = "rt_context.json";

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
}

impl BuildSharedState {
    #[must_use]
    const fn new(
        force_reinstall: bool,
        no_editable: bool,
        riot_root: PathBuf,
        project_root: PathBuf,
    ) -> Self {
        Self {
            force_reinstall,
            no_editable,
            riot_root,
            project_root,
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

        let uv_project = ensure_uv_project(&self.riot_root, &self.project_root, venv, exc)?;

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

        let status = cmd.status()?;

        if !status.success() {
            return Err(RtError::message(format!(
                "error: uv sync failed with status {status}"
            )));
        }

        let site_packages_path =
            exc_venv_path.join(format!("lib/python{}/site-packages", venv.python));

        self.configure_site_packages(venv, exc, &site_packages_path)?;

        fs::write(exc_venv_path.join(DONE_MARKER), format!("{RT_VERSION}\n"))?;

        Ok(StepOutcome::Done)
    }

    fn configure_site_packages(
        &self,
        venv: &RiotVenv,
        exc: &ExecutionContext,
        site_packages_path: &Path,
    ) -> RtResult<()> {
        fs::create_dir_all(site_packages_path)?;

        let project_root =
            fs::canonicalize(&self.project_root).unwrap_or_else(|_| self.project_root.clone());

        fs::write(
            site_packages_path.join("pytest_rt.py"),
            include_str!("../../pytest_rt/pytest_rt.py"),
        )?;
        // envier is a ddtrace runtime dependency, so it lands in the venv
        // through the local project dependency rather than through a riotfile
        // package. A context that skips the project only has it when the
        // riotfile asks for it explicitly.
        if !exc.skip_dev_install || has_pkg(venv, "envier") {
            write_envier_stubs(site_packages_path)?;
        }

        self.write_runtime_context(
            site_packages_path,
            &project_root,
            exc,
            &venv.services,
            &venv.suite_env,
        )?;

        Ok(())
    }

    fn write_runtime_context(
        &self,
        site_packages_path: &Path,
        project_root: &Path,
        exc: &ExecutionContext,
        services: &[String],
        suite_env: &IndexMap<String, String>,
    ) -> RtResult<()> {
        let mut environment = suite_env.clone();

        // Values from riotfile.py override inherited suitespec values.
        environment.extend(exc.env.clone());

        let project_root = project_root.to_string_lossy().into_owned();
        let manifest = RuntimeContextManifest {
            project_root: &project_root,
            no_editable: self.no_editable,
            environment,
            services,
            command: exc.command.as_deref(),
        };
        let mut manifest = serde_json::to_string_pretty(&manifest).map_err(|err| {
            RtError::message(format!("error: failed to serialize runtime context: {err}"))
        })?;
        manifest.push('\n');

        fs::write(site_packages_path.join(CONTEXT_MANIFEST), manifest)?;
        fs::write(
            site_packages_path.join("sitecustomize.py"),
            include_str!("../../pytest_rt/sitecustomize.py"),
        )?;

        Ok(())
    }
}

#[derive(Serialize)]
struct RuntimeContextManifest<'a> {
    project_root: &'a str,
    no_editable: bool,
    environment: IndexMap<String, String>,
    services: &'a [String],
    command: Option<&'a str>,
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
    use super::{BuildSharedState, CONTEXT_MANIFEST, ENVIER_STUBS, write_envier_stubs};
    use crate::venv::{ExecutionContext, RiotVenv};
    use indexmap::IndexMap;
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
    fn runtime_context_uses_static_python_files_and_a_json_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let project_root = dir.path().join("project");
        let site_packages = dir.path().join("site-packages");
        fs::create_dir_all(&project_root).unwrap();
        fs::create_dir_all(&site_packages).unwrap();
        let state =
            BuildSharedState::new(false, true, dir.path().join(".riot"), project_root.clone());
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
            .configure_site_packages(&venv, &exc, &site_packages)
            .unwrap();

        assert_eq!(
            fs::read_to_string(site_packages.join("sitecustomize.py")).unwrap(),
            include_str!("../../pytest_rt/sitecustomize.py")
        );
        assert_eq!(
            fs::read_to_string(site_packages.join("pytest_rt.py")).unwrap(),
            include_str!("../../pytest_rt/pytest_rt.py")
        );
        let manifest: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(site_packages.join(CONTEXT_MANIFEST)).unwrap(),
        )
        .unwrap();
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
    }
}
