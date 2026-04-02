use std::{fmt::Write as _, fs, path::Path};

use crate::{
    constants::VENV_PREFIX,
    error::{RtError, RtResult},
    ui,
};

/// Remove virtual environments created under the riot root while keeping compiled requirements.
///
/// # Errors
///
/// Returns an error if directory traversal or removal fails.
pub fn run(riot_root: &Path) -> RtResult<()> {
    ui::step(format!(
        "Cleaning virtual environments under {}",
        riot_root.display()
    ));

    if !riot_root.exists() {
        ui::detail("Riot root not found, nothing to clean.");
        ui::blank_line();
        return Ok(());
    }

    let mut targets = Vec::new();
    let entries = fs::read_dir(riot_root).map_err(|err| {
        RtError::message(format!(
            "error: failed to read riot root {}: {err}",
            riot_root.display()
        ))
    })?;

    for entry_result in entries {
        let entry = entry_result.map_err(|err| {
            RtError::message(format!("error: failed to evaluate riot root entry: {err}"))
        })?;
        let file_type = entry.file_type().map_err(|err| {
            RtError::message(format!(
                "error: could not inspect entry {}: {err}",
                entry.path().display()
            ))
        })?;

        if !file_type.is_dir() {
            continue;
        }

        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == "requirements" {
            continue;
        }
        if !name.starts_with(VENV_PREFIX) {
            continue;
        }

        targets.push(entry.path());
    }

    if targets.is_empty() {
        ui::detail("No cached virtual environments were found.");
        ui::blank_line();
        return Ok(());
    }

    targets.sort();

    let mut failures = Vec::new();

    for target in &targets {
        ui::detail(format!("Removing {}", target.display()));
        if let Err(err) = fs::remove_dir_all(target) {
            failures.push((target.display().to_string(), err));
        }
    }

    ui::blank_line();

    if failures.is_empty() {
        Ok(())
    } else {
        let mut message =
            String::from("error: failed to remove the following directories while cleaning:");
        for (path, err) in failures {
            let _ = write!(&mut message, "\n- {path}: {err}");
        }
        Err(RtError::message(message))
    }
}
