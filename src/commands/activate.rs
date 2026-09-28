use indexmap::IndexMap;

use crate::{
    commands::{build::build_selected_environments, shell::resolve_target},
    config::RepoConfig,
    error::RtResult,
    ui,
    venv::{self, Suite},
};

/// Build the requested environment and print the activation script path.
///
/// # Errors
///
/// Returns an error if the target cannot be resolved or the environment build fails.
pub fn run(
    venvs: &IndexMap<String, Suite>,
    repo: &RepoConfig,
    hash: &str,
    force_reinstall: bool,
) -> RtResult<()> {
    let target = resolve_target(venvs, hash)?;
    let run = &target.runs[0];
    build_selected_environments(repo, std::slice::from_ref(&target), force_reinstall, false)?;
    let activation_path =
        venv::profile_path(&repo.cache_root, &target.hash, run.index).join("bin/activate");

    ui::step(format!(
        "To activate the chosen venv use `source $(rt activate {hash})`"
    ));

    println!("{}", activation_path.display());

    Ok(())
}
