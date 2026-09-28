use std::fs;

use indexmap::IndexMap;
use std::os::unix::fs::symlink;

use crate::{
    commands::{build::build_selected_environments, shell::resolve_target},
    config::RepoConfig,
    error::{RtError, RtResult},
    venv::{self, Suite},
};

/// Build the requested test profile and link it as `.venv` in the project root.
///
/// # Errors
///
/// Returns an error if target resolution, build, or link replacement fails.
pub fn run(
    venvs: &IndexMap<String, Suite>,
    repo: &RepoConfig,
    hash: &str,
    force_reinstall: bool,
) -> RtResult<()> {
    let target = resolve_target(venvs, hash)?;
    let run = &target.runs[0];

    build_selected_environments(repo, std::slice::from_ref(&target), force_reinstall, false)?;

    let project_root = &repo.project_root;

    let venv_dir = venv::profile_path(&repo.cache_root, &target.hash, run.index);
    let venv_dir = fs::canonicalize(&venv_dir).unwrap_or(venv_dir);
    let link_path = project_root.join(".venv");

    if let Ok(metadata) = fs::symlink_metadata(&link_path) {
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            fs::remove_dir_all(&link_path).map_err(|err| {
                RtError::message(format!(
                    "error: failed to remove existing .venv directory: {err}"
                ))
            })?;
        } else {
            fs::remove_file(&link_path).map_err(|err| {
                RtError::message(format!(
                    "error: failed to remove existing .venv link: {err}"
                ))
            })?;
        }
    }

    symlink(&venv_dir, &link_path).map_err(|err| {
        RtError::message(format!(
            "error: failed to create .venv symlink ({} -> {}): {err}",
            link_path.display(),
            venv_dir.display()
        ))
    })?;

    Ok(())
}
