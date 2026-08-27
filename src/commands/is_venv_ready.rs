use std::{fs, path::Path};

use crate::{
    constants::{DONE_MARKER, RT_VERSION},
    error::{RtError, RtResult},
    venv::{venv_path, venv_python_path},
};

/// Report whether an execution-context environment was completely built by
/// this version of rt.
///
/// This is an internal command used by integrations. Keeping the readiness
/// policy here prevents callers from depending on rt's marker-file layout.
pub fn run(riot_root: &Path, hash: &str) -> RtResult<()> {
    if hash.is_empty()
        || !hash.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '@' | '_' | '-')
        })
    {
        return Err(RtError::with_code(2, "error: invalid venv hash"));
    }

    println!("{}", is_ready(riot_root, hash));
    Ok(())
}

fn is_ready(riot_root: &Path, hash: &str) -> bool {
    let environment_path = venv_path(riot_root, hash);
    let marker_is_current = fs::read_to_string(environment_path.join(DONE_MARKER))
        .is_ok_and(|contents| contents.trim() == RT_VERSION);

    marker_is_current && Path::new(&venv_python_path(riot_root, hash)).is_file()
}

#[cfg(test)]
mod tests {
    use super::is_ready;
    use crate::{
        constants::{DONE_MARKER, RT_VERSION},
        venv::venv_path,
    };
    use std::fs;

    #[test]
    fn readiness_requires_python_and_a_current_completion_marker() {
        let root = tempfile::tempdir().unwrap();
        let hash = "abc1234@def5678";
        let environment = venv_path(root.path(), hash);
        fs::create_dir_all(environment.join("bin")).unwrap();

        assert!(!is_ready(root.path(), hash));

        fs::write(environment.join("bin/python"), "").unwrap();
        assert!(!is_ready(root.path(), hash));

        fs::write(environment.join(DONE_MARKER), "an-older-version\n").unwrap();
        assert!(!is_ready(root.path(), hash));

        fs::write(environment.join(DONE_MARKER), format!("{RT_VERSION}\n")).unwrap();
        assert!(is_ready(root.path(), hash));
    }
}
