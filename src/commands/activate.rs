use std::path::Path;

use pyo3::{PyResult, Python};

use crate::{
    commands::{build::build_selected_contexts, shell::resolve_target},
    config::RepoConfig,
    ui, venv,
};

/// Build the requested environment and print the activation script path.
pub fn run(py: Python<'_>, repo: &RepoConfig, hash: &str, force_reinstall: bool) -> PyResult<()> {
    let target = resolve_target(py, &repo.riotfile_path, hash)?;
    let ctx_hash = &target.execution_contexts[0].hash;
    build_selected_contexts(repo, std::slice::from_ref(&target), force_reinstall)?;
    let activation_path = activation_path(ctx_hash, &repo.riot_root);

    ui::step(format!(
        "To activate the chose venv use `source $(rt activate {hash})"
    ));

    println!("{activation_path}");

    Ok(())
}

fn activation_path(hash: &str, riot_root: &Path) -> String {
    let venv_dir = venv::venv_path(riot_root, hash);
    let script = if cfg!(windows) {
        venv_dir.join("Scripts/activate")
    } else {
        venv_dir.join("bin/activate")
    };
    script.to_string_lossy().into_owned()
}
