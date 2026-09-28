use std::{fs, path::Path};

use crate::{
    error::{RtError, RtResult},
    ui,
};

pub fn run(cache_root: &Path) -> RtResult<()> {
    ui::step(format!("Cleaning rt cache under {}", cache_root.display()));
    if !cache_root.exists() {
        ui::detail("Cache not found, nothing to clean.");
        return Ok(());
    }
    fs::remove_dir_all(cache_root).map_err(|error| {
        RtError::message(format!(
            "error: failed to remove {}: {error}",
            cache_root.display()
        ))
    })?;
    Ok(())
}
