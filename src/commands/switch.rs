use std::fs;

use indexmap::IndexMap;
use pyo3::{exceptions::PySystemExit, PyErr, PyResult};
use std::os::unix::fs::symlink;

use crate::{
    commands::{build::build_selected_contexts, shell::resolve_target},
    config::RepoConfig,
    venv::{self, RiotVenv},
};

/// Build the requested environment and link it as `.venv` in the riotfile directory.
///
/// # Errors
///
/// Returns an error if target resolution, build, or link replacement fails.
pub fn run(
    venvs: IndexMap<String, RiotVenv>,
    repo: &RepoConfig,
    hash: &str,
    force_reinstall: bool,
) -> PyResult<()> {
    let target = resolve_target(venvs, hash)?;
    let ctx_hash = &target.execution_contexts[0].hash;

    build_selected_contexts(repo, std::slice::from_ref(&target), force_reinstall)?;

    let project_root = repo.riotfile_path.parent().ok_or_else(|| {
        eprintln!("error: could not determine riotfile parent directory");
        PyErr::new::<PySystemExit, _>(1)
    })?;

    let venv_dir = venv::venv_path(&repo.riot_root, ctx_hash);
    let venv_dir = fs::canonicalize(&venv_dir).unwrap_or(venv_dir);
    let link_path = project_root.join(".venv");

    if let Ok(metadata) = fs::symlink_metadata(&link_path) {
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            fs::remove_dir_all(&link_path).map_err(|err| {
                eprintln!("error: failed to remove existing .venv directory: {err}");
                PyErr::new::<PySystemExit, _>(1)
            })?;
        } else {
            fs::remove_file(&link_path).map_err(|err| {
                eprintln!("error: failed to remove existing .venv link: {err}");
                PyErr::new::<PySystemExit, _>(1)
            })?;
        }
    }

    symlink(&venv_dir, &link_path).map_err(|err| {
        eprintln!(
            "error: failed to create .venv symlink ({} -> {}): {err}",
            link_path.display(),
            venv_dir.display()
        );
        PyErr::new::<PySystemExit, _>(1)
    })?;

    Ok(())
}
