use indexmap::IndexMap;

use crate::{
    commands::{build::build_selected_contexts, shell::resolve_target},
    config::RepoConfig,
    error::RtResult,
    ui,
    venv::{self, RiotVenv},
};

/// Build the requested environment and print the activation script path.
///
/// # Errors
///
/// Returns an error if the target cannot be resolved or the environment build fails.
pub fn run(
    venvs: IndexMap<String, RiotVenv>,
    repo: &RepoConfig,
    hash: &str,
    force_reinstall: bool,
) -> RtResult<()> {
    let target = resolve_target(venvs, hash)?;
    let ctx_hash = &target.execution_contexts[0].hash;
    build_selected_contexts(repo, std::slice::from_ref(&target), force_reinstall, false)?;
    let venv_dir = venv::venv_path(&repo.riot_root, ctx_hash);
    let activation_path = if cfg!(windows) {
        venv_dir.join("Scripts/activate")
    } else {
        venv_dir.join("bin/activate")
    };

    ui::step(format!(
        "To activate the chosen venv use `source $(rt activate {hash})`"
    ));

    println!("{}", activation_path.display());

    Ok(())
}
