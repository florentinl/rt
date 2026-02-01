use std::{
    collections::{HashMap, HashSet},
    error::Error,
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::{
    command::ManagedCommand,
    config::Selector,
    progress::{
        summarize_errors, MultiplexedProgressLogger, ProgressLogger, StepContext, StepId,
        StepOutcome, Task, TaskRunner,
    },
    venv::{venv_path, ExecutionContext, RiotVenv},
};
use indexmap::IndexMap;
use itertools::Itertools;
use tempfile::{Builder, NamedTempFile};

use pyo3::{exceptions::PySystemExit, PyErr, PyResult, Python};
use rayon::current_num_threads;

use crate::{
    config::RepoConfig,
    constants::{DONE_MARKER, REQUIREMENTS_DIR, VENV_DEPS_DIR, VENV_SELF_DIR},
    venv::select_execution_contexts,
};

/// Build the virtual environment for the provided execution context.
pub fn run(
    py: Python<'_>,
    repo: &RepoConfig,
    selector: Selector,
    force_reinstall: bool,
) -> PyResult<()> {
    let selected = select_execution_contexts(py, &repo.riotfile_path, selector)?;
    build_selected_contexts(repo, &selected, force_reinstall)?;
    Ok(())
}

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

pub fn build_selected_contexts(
    repo: &RepoConfig,
    selected: &[RiotVenv],
    force_reinstall: bool,
) -> PyResult<()> {
    if let Err(e) = fs::DirBuilder::new()
        .recursive(true)
        .create(&repo.riot_root)
    {
        eprintln!("error: could not create riot root: {e}");
        return Err(PyErr::new::<PySystemExit, _>(1));
    }
    let sink: Arc<dyn ProgressLogger> = Arc::new(MultiplexedProgressLogger::new().unwrap());
    let shared = Arc::new(BuildSharedState::new(
        force_reinstall,
        Arc::clone(&repo.build_env),
        Arc::clone(&repo.run_env),
        repo.riot_root.clone(),
        repo.pytest_plugin_dir.clone(),
    ));
    let runner = TaskRunner::new(Arc::clone(&sink)).with_parallelism(Some(current_num_threads()));

    let context_indices = collect_context_indices(selected);

    let mut dev_pythons: HashSet<String> = HashSet::new();
    let mut deps_targets: HashSet<usize> = HashSet::new();
    for (venv_idx, ctx_idx) in &context_indices {
        let selected_venv = &selected[*venv_idx];
        let exc = &selected_venv.execution_contexts[*ctx_idx];
        if !exc.skip_dev_install {
            dev_pythons.insert(selected_venv.python.clone());
        }
        deps_targets.insert(*venv_idx);
    }

    let mut setup_tasks: Vec<_> = Vec::new();
    setup_tasks.extend(dev_pythons.into_iter().map(|python| {
        let state = Arc::clone(&shared);
        let step_id = format!("dev install {python}");
        Task::new(StepId::new(&step_id), &step_id, move |ctx| {
            state.ensure_dev_install(&python, &ctx)
        })
    }));

    setup_tasks.extend(deps_targets.into_iter().map(|idx| {
        let state = Arc::clone(&shared);
        let venv = &selected[idx];
        let step_id = format!("deps install {}", venv.hash);
        Task::new(StepId::new(&step_id), &step_id, move |ctx| {
            state.ensure_deps_install(venv, &ctx)
        })
    }));

    let exc_ctx_tasks: Vec<_> = context_indices
        .iter()
        .map(|&(venv_i, exc_i)| {
            let state = Arc::clone(&shared);
            let venv = &selected[venv_i];
            let exc_ctx = &venv.execution_contexts[exc_i];
            let step_id = format!("create execution context {}", exc_ctx.hash);
            Task::new(StepId::new(&step_id), &step_id, move |ctx| {
                state.ensure_execution_ctx(venv, exc_ctx, &ctx)
            })
        })
        .collect();

    let setup_errors = runner.run(setup_tasks).map_err(|err| {
        eprintln!("error: could not configure build parallelism ({err})");
        PyErr::new::<PySystemExit, _>(1)
    })?;

    if summarize_errors(&setup_errors, "build") {
        return Err(PyErr::new::<PySystemExit, _>(1));
    }

    let exc_errors = runner.run(exc_ctx_tasks).map_err(|err| {
        eprintln!("error: could not configure build parallelism ({err})");
        PyErr::new::<PySystemExit, _>(1)
    })?;

    if summarize_errors(&exc_errors, "build") {
        return Err(PyErr::new::<PySystemExit, _>(1));
    }

    Ok(())
}

type DynError = Box<dyn Error + Send + Sync>;
type DynResult<T> = Result<T, DynError>;

pub struct BuildSharedState {
    force_reinstall: bool,
    build_env: Arc<HashMap<String, String>>,
    run_env: Arc<HashMap<String, String>>,
    riot_root: PathBuf,
    pytest_plugin_dir: PathBuf,
}

impl BuildSharedState {
    pub fn new(
        force_reinstall: bool,
        build_env: Arc<HashMap<String, String>>,
        run_env: Arc<HashMap<String, String>>,
        riot_root: PathBuf,
        pytest_plugin_dir: PathBuf,
    ) -> Self {
        Self {
            force_reinstall,
            build_env,
            run_env,
            riot_root,
            pytest_plugin_dir,
        }
    }

    fn ensure_dev_install(&self, python: &str, ctx: &StepContext) -> DynResult<StepOutcome> {
        let dev_install_path = get_dev_install_path(&self.riot_root, python);

        let marker_path = dev_install_path.join(DONE_MARKER);
        if !self.force_reinstall && marker_path.is_file() {
            return Ok(StepOutcome::Cached);
        }

        if dev_install_path.exists() {
            fs::remove_dir_all(&dev_install_path)?;
        }
        fs::create_dir_all(&dev_install_path)?;

        let status = ManagedCommand::new_uv("pip", Arc::clone(&ctx.sink), ctx.step_id.clone())
            .envs(self.build_env.as_ref())
            .arg("install")
            .arg("-v")
            .arg("--system")
            .arg("--python")
            .arg(python)
            .arg("--target")
            .arg(&dev_install_path)
            .args(["-e", "."])
            .args(["--config-setting", "editable_mode=compat"])
            .status()?;

        if !status.success() {
            return Err(Box::new(io::Error::other(format!(
                "uv pip install failed with status {status}"
            ))));
        }

        File::create(marker_path)?;

        Ok(StepOutcome::Done)
    }

    fn get_requirements_file(&self, venv: &RiotVenv) -> DynResult<NamedTempFile> {
        let requirements_txt = self
            .riot_root
            .join(REQUIREMENTS_DIR)
            .join(format!("{}.txt", venv.hash));

        let requirements = if requirements_txt.exists() {
            fs::read_to_string(requirements_txt)?
        } else {
            format_requirements(&venv.pkgs)
        }
        .replace("/home/bits/project", ".");

        let mut temp = Builder::new().suffix(".txt").tempfile()?;
        temp.write_all(requirements.as_bytes())?;
        temp.flush()?;

        Ok(temp)
    }

    fn ensure_deps_install(&self, venv: &RiotVenv, ctx: &StepContext) -> DynResult<StepOutcome> {
        let mut deps_install_path = self.riot_root.clone();
        deps_install_path.push(VENV_DEPS_DIR);
        deps_install_path.push(format!("deps_{}", venv.hash));

        let requirements_file = self.get_requirements_file(venv)?;

        let marker_path = deps_install_path.join(DONE_MARKER);
        if !self.force_reinstall && marker_path.is_file() {
            return Ok(StepOutcome::Cached);
        }

        if deps_install_path.exists() {
            fs::remove_dir_all(&deps_install_path)?;
        }
        fs::create_dir_all(&deps_install_path)?;

        let status = ManagedCommand::new_uv("pip", Arc::clone(&ctx.sink), ctx.step_id.clone())
            .envs(self.build_env.as_ref())
            .arg("install")
            .arg("--system")
            .arg("--python")
            .arg(&venv.python)
            .arg("--target")
            .arg(&deps_install_path)
            .arg("--requirement")
            .arg(&requirements_file.path())
            .status()?;

        if !status.success() {
            return Err(Box::new(io::Error::other(format!(
                "uv pip install failed with status {status}"
            ))));
        }

        File::create(marker_path)?;

        Ok(StepOutcome::Done)
    }

    fn ensure_execution_ctx(
        &self,
        venv: &RiotVenv,
        exc: &ExecutionContext,
        ctx: &StepContext,
    ) -> DynResult<StepOutcome> {
        let exc_venv_path = venv_path(&self.riot_root, &exc.hash);
        let marker_path = exc_venv_path.join(DONE_MARKER);

        if !self.force_reinstall && marker_path.is_file() {
            return Ok(StepOutcome::Cached);
        }

        if marker_path.is_file() {
            fs::remove_file(&marker_path)?;
        }

        let deps_install_path = get_deps_install_path(&self.riot_root, &venv.hash);
        let dev_install_path =
            (!exc.skip_dev_install).then_some(get_dev_install_path(&self.riot_root, &venv.python));

        let status = ManagedCommand::new_uv("venv", Arc::clone(&ctx.sink), ctx.step_id.clone())
            .envs(self.build_env.as_ref())
            .arg("--python")
            .arg(&venv.python)
            .arg("--clear")
            .arg(&exc_venv_path)
            .status()?;

        if !status.success() {
            return Err(Box::new(io::Error::other(format!(
                "uv venv failed with status {status}"
            ))));
        }

        let site_packages_path =
            exc_venv_path.join(format!("lib/python{}/site-packages", &venv.python));
        self.configure_site_packages(
            exc,
            &deps_install_path,
            dev_install_path.as_ref(),
            &site_packages_path,
        )?;

        let mut bin_sources: Vec<&Path> = Vec::new();
        if let Some(ref dev_install_path) = dev_install_path {
            bin_sources.push(dev_install_path);
        }
        bin_sources.push(&deps_install_path);
        merge_bin_dirs(&exc_venv_path, &bin_sources).map_err(|err| Box::new(err) as DynError)?;

        File::create(marker_path)?;

        Ok(StepOutcome::Done)
    }

    fn configure_site_packages(
        &self,
        exc: &ExecutionContext,
        deps_install_path: &Path,
        dev_install_path: Option<&PathBuf>,
        site_packages_path: &Path,
    ) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
        fs::create_dir_all(site_packages_path)?;

        let current_dir = self.riot_root.parent().ok_or_else(|| {
            eprintln!("error: could not find riot root parent");
            return PyErr::new::<PySystemExit, _>(1);
        })?;

        let mut paths = vec![];
        if let Some(dev_install_path) = dev_install_path {
            paths.push((current_dir.to_string_lossy(), "current project"));
            paths.push((
                dev_install_path.to_string_lossy(),
                "current project dependencies",
            ));
        }
        paths.push((deps_install_path.to_string_lossy(), "riot dependencies"));
        paths.push((
            self.pytest_plugin_dir.to_string_lossy(),
            "pytest plugin injected by rt",
        ));

        {
            // pth file is necessary for analysis tools that do not execute sitecustomize.py
            let pth_file = site_packages_path.join("riot.pth");
            let pth_content = paths
                .iter()
                .map(|(path, comment)| format!("# {}\n{}", comment, path))
                .join("\n");
            fs::write(pth_file, pth_content)?;
        }

        {
            let sitecustomize_path = site_packages_path.join("sitecustomize.py");
            let mut sitecustomize_content = String::new();
            sitecustomize_content.push_str("import site, os\n");

            // adding paths in sitecustomize is necessary to include transitive .pth files
            for (path, comment) in paths {
                sitecustomize_content
                    .push_str(&format!("site.addsitedir(r\"{}\") # {}\n", path, comment));
            }

            // environment variables from riotfile.py
            if !exc.env.is_empty() {
                sitecustomize_content.push_str("\n# Environment variables from riotfile.py\n");
                for (key, val) in &exc.env {
                    sitecustomize_content
                        .push_str(&format!("os.environ[r\"{}\"] = r\"{}\"\n", key, val));
                }
            }

            // environment variables from rt.toml
            if !self.run_env.is_empty() {
                sitecustomize_content.push_str("\n# Environment variables from rt.toml\n");
                for (key, val) in self.run_env.iter() {
                    sitecustomize_content
                        .push_str(&format!("os.environ[r\"{}\"] = r\"{}\"\n", key, val));
                }
            }

            // enable pytest rt
            sitecustomize_content.push_str("\n# Enable pytest_rt plugin\n");
            sitecustomize_content.push_str("from enable_pytest_rt import enable\n");
            sitecustomize_content.push_str("enable()\n");

            fs::write(sitecustomize_path, sitecustomize_content)?;
        }

        Ok(())
    }
}

fn get_dev_install_path(riot_root: &Path, python: &str) -> PathBuf {
    let python_no_dot = python.replace('.', "");
    let mut dev_install_path = riot_root.to_path_buf();
    dev_install_path.push(VENV_SELF_DIR);
    dev_install_path.push(format!("self_py{python_no_dot}"));
    dev_install_path
}

fn get_deps_install_path(riot_root: &Path, hash: &str) -> PathBuf {
    let mut deps_install_path = riot_root.to_path_buf();
    deps_install_path.push(VENV_DEPS_DIR);
    deps_install_path.push(format!("deps_{hash}"));
    deps_install_path
}

fn format_requirements(pkgs: &IndexMap<String, String>) -> String {
    if pkgs.is_empty() {
        return String::new();
    }

    let mut buf = String::new();
    for (idx, (lib, version)) in pkgs.iter().enumerate() {
        if idx > 0 {
            buf.push('\n');
        }
        buf.push_str(lib);
        buf.push_str(version);
    }
    buf.push('\n');
    buf
}

fn merge_bin_dirs(venv_path: &Path, sources: &[&Path]) -> io::Result<()> {
    let target_bin = venv_path.join("bin");
    let absolute_venv = fs::canonicalize(venv_path).unwrap_or_else(|_| venv_path.to_path_buf());
    let python_exe = absolute_venv.join("bin/python");
    let python_shebang = format!("#!{}\n", python_exe.to_string_lossy());

    for source in sources {
        let source_bin = source.join("bin");
        if !source_bin.exists() {
            continue;
        }
        for entry in fs::read_dir(&source_bin)? {
            let entry = entry?;
            let path = entry.path();
            let metadata = entry.metadata()?;
            if metadata.is_dir() {
                continue;
            }
            let target = target_bin.join(entry.file_name());
            if target.exists() {
                fs::remove_file(&target)?;
            }

            let content = fs::read(&path)?;
            let rewritten = rewrite_python_shebang(&content, &python_shebang);
            let data = rewritten.as_ref().unwrap_or(&content);
            fs::write(&target, data)?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = metadata.permissions().mode();
                fs::set_permissions(&target, fs::Permissions::from_mode(mode))?;
            }
        }
    }
    Ok(())
}

fn rewrite_python_shebang(content: &[u8], python_shebang: &str) -> Option<Vec<u8>> {
    if !content.starts_with(b"#!") {
        return None;
    }

    let line_end = content
        .iter()
        .position(|&b| b == b'\n')
        .unwrap_or(content.len());
    let shebang_line = &content[..line_end];
    let shebang_str = std::str::from_utf8(shebang_line).ok()?;

    if !shebang_str.to_ascii_lowercase().contains("python") {
        return None;
    }

    let rest_start = if line_end < content.len() {
        line_end + 1
    } else {
        content.len()
    };
    let rest = &content[rest_start..];

    let mut rewritten = Vec::with_capacity(python_shebang.len() + rest.len());
    rewritten.extend_from_slice(python_shebang.as_bytes());
    rewritten.extend_from_slice(rest);
    Some(rewritten)
}
