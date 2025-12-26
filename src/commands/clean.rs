use std::{fmt::Write as _, fs, path::Path};

use pyo3::{exceptions::PySystemExit, PyErr, PyResult};

use crate::{constants::VENV_PREFIX, ui};

/// Remove virtual environments created under the riot root while keeping compiled requirements.
pub fn run(riot_root: &Path) -> PyResult<()> {
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
        eprintln!(
            "error: failed to read riot root {}: {err}",
            riot_root.display()
        );
        PyErr::new::<PySystemExit, _>(1)
    })?;

    for entry_result in entries {
        let entry = entry_result.map_err(|err| {
            eprintln!("error: failed to evaluate riot root entry: {err}");
            PyErr::new::<PySystemExit, _>(1)
        })?;
        let file_type = entry.file_type().map_err(|err| {
            eprintln!(
                "error: could not inspect entry {}: {err}",
                entry.path().display()
            );
            PyErr::new::<PySystemExit, _>(1)
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
        eprintln!("{message}");
        Err(PyErr::new::<PySystemExit, _>(1))
    }
}
