use std::{fs, path::Path};

use serde::Deserialize;

use crate::{
    constants::{RT_VERSION, RUNTIME_CONFIG},
    error::{RtError, RtResult},
    venv::{environment_python_path, profile_path},
};

#[derive(Deserialize)]
struct RuntimeProfile {
    rt_version: String,
    profile_fingerprint: String,
}

pub fn run(cache_root: &Path, id: &str, expected_fingerprint: Option<&str>) -> RtResult<()> {
    if id.is_empty()
        || !id.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '@' | '_' | '-')
        })
    {
        return Err(RtError::with_code(
            2,
            "error: invalid environment or run id",
        ));
    }
    println!("{}", is_ready(cache_root, id, expected_fingerprint));
    Ok(())
}

fn is_ready(cache_root: &Path, id: &str, expected_fingerprint: Option<&str>) -> bool {
    let Some((hash, index)) = id.split_once('@') else {
        return expected_fingerprint.is_none() && environment_python_path(cache_root, id).is_file();
    };
    let Ok(index) = index.parse::<usize>() else {
        return false;
    };
    let profile = profile_path(cache_root, hash, index);
    let current = fs::read_to_string(profile.join(RUNTIME_CONFIG))
        .ok()
        .and_then(|contents| serde_json::from_str::<RuntimeProfile>(&contents).ok())
        .is_some_and(|manifest| {
            manifest.rt_version == RT_VERSION
                && expected_fingerprint
                    .is_none_or(|expected| manifest.profile_fingerprint == expected)
        });
    current
        && environment_python_path(cache_root, hash).is_file()
        && profile.join("bin/python").is_file()
        && profile.join("bin/activate").is_file()
}

#[cfg(test)]
mod tests {
    use super::is_ready;
    use crate::{
        constants::{RT_VERSION, RUNTIME_CONFIG},
        venv::{environment_path, profile_path},
    };
    use std::fs;

    #[test]
    fn run_readiness_requires_shared_python_and_current_profile() {
        let root = tempfile::tempdir().unwrap();
        let physical = environment_path(root.path(), "abc1234");
        let profile = profile_path(root.path(), "abc1234", 0);
        fs::create_dir_all(physical.join("bin")).unwrap();
        fs::create_dir_all(profile.join("bin")).unwrap();
        fs::write(physical.join("bin/python"), "").unwrap();
        fs::write(profile.join("bin/python"), "").unwrap();
        fs::write(profile.join("bin/activate"), "").unwrap();
        fs::write(
            profile.join(RUNTIME_CONFIG),
            format!("{{\"rt_version\":\"{RT_VERSION}\",\"profile_fingerprint\":\"current\"}}"),
        )
        .unwrap();
        assert!(is_ready(root.path(), "abc1234@0", Some("current")));
        assert!(!is_ready(root.path(), "abc1234@0", Some("stale")));
        assert!(!is_ready(root.path(), "abc1234@1", Some("current")));
    }
}
