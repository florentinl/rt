use std::{
    collections::{HashMap, HashSet},
    error::Error,
    ffi::OsStr,
    fmt::Write as FmtWrite,
    fs::{self, File},
    io::{self, BufWriter, Write},
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
    pytest_plugin_dir: Option<PathBuf>,
}

impl BuildSharedState {
    pub fn new(
        force_reinstall: bool,
        build_env: Arc<HashMap<String, String>>,
        run_env: Arc<HashMap<String, String>>,
        riot_root: PathBuf,
        pytest_plugin_dir: Option<PathBuf>,
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
            // .args(["--config-setting", "editable_mode=compat"])
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
        self.write_sitecustomize(
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

    fn write_sitecustomize(
        &self,
        exc: &ExecutionContext,
        deps_install_path: &Path,
        dev_install_path: Option<&PathBuf>,
        site_packages_path: &Path,
    ) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
        fs::create_dir_all(site_packages_path)?;
        let sitecustomize_path = site_packages_path.join("sitecustomize.py");
        let sitecustomize_file = File::create(sitecustomize_path)?;
        let mut sitecustomize = BufWriter::new(sitecustomize_file);
        {
            if let Some(dev_install_path) = dev_install_path {
                let dev_install_pth_path = site_packages_path.join("self-deps.pth");
                let current_dir = self.riot_root.parent().ok_or_else(|| {
                    eprintln!("error: could not find riot root parent");
                    return PyErr::new::<PySystemExit, _>(1);
                })?;
                fs::write(
                    &dev_install_pth_path,
                    format!(
                        "{}\n{}\n",
                        dev_install_path.to_string_lossy(),
                        current_dir.to_string_lossy()
                    ),
                )?;

                for entry in fs::read_dir(dev_install_path)? {
                    let entry = entry?;
                    let file_name = entry.file_name();
                    if !file_name
                        .to_str()
                        .map(|name| name.starts_with("__editable__"))
                        .unwrap_or(false)
                    {
                        continue;
                    }
                    if entry.file_type()?.is_file() {
                        fs::copy(entry.path(), site_packages_path.join(file_name))?;
                    }
                }
            }
        }
        {
            let deps_pth_path = site_packages_path.join("riot-deps.pth");
            fs::write(
                &deps_pth_path,
                format!("{}\n", deps_install_path.to_string_lossy()),
            )?;
        }
        {
            writeln!(sitecustomize, "# Environment variables from riotfile.py")?;
            writeln!(sitecustomize, "import os")?;
            for (key, val) in &exc.env {
                writeln!(
                    sitecustomize,
                    "os.environ[{}] = {}",
                    python_string_literal(key),
                    python_string_literal(val)
                )?;
            }
        }
        writeln!(sitecustomize)?;
        {
            writeln!(sitecustomize, "# Environment variables from rt.toml")?;
            for (key, val) in self.run_env.as_ref() {
                writeln!(
                    sitecustomize,
                    "os.environ[{}] = {}",
                    python_string_literal(key),
                    python_string_literal(val)
                )?;
            }
        }
        writeln!(sitecustomize)?;
        {
            if let Some(plugin_dir) = self
                .pytest_plugin_dir
                .as_ref()
                .filter(|plugin_dir| plugin_dir.is_dir())
            {
                fs::write(
                    site_packages_path.join("pytest-rt.pth"),
                    format!("{}\n", plugin_dir.to_string_lossy()),
                )?;
                writeln!(sitecustomize, "# rt pytest plugin")?;
                writeln!(
                sitecustomize,
                "plugins = [p.strip() for p in os.environ.get('PYTEST_PLUGINS', '').split(',') if p.strip()]"
            )?;
                writeln!(sitecustomize, "if \"rt\" not in plugins:",)?;
                writeln!(sitecustomize, "    plugins.append(\"rt\")",)?;
                writeln!(
                    sitecustomize,
                    "os.environ['PYTEST_PLUGINS'] = ','.join(plugins)"
                )?;
                writeln!(sitecustomize)?;
            }
        }
        sitecustomize.flush()?;
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

fn python_string_literal<S: AsRef<OsStr>>(value: S) -> String {
    let value = value.as_ref().to_str().unwrap();
    let mut literal = String::with_capacity(value.len() + 2);
    literal.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => literal.push_str("\\\\"),
            '"' => literal.push_str("\\\""),
            '\n' => literal.push_str("\\n"),
            '\r' => literal.push_str("\\r"),
            '\t' => literal.push_str("\\t"),
            ch if ch.is_control() => {
                literal.push_str("\\x");
                FmtWrite::write_fmt(&mut literal, format_args!("{:02x}", ch as u32))
                    .expect("write to string");
            }
            ch => literal.push(ch),
        }
    }
    literal.push('"');
    literal
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
