//! Constants used throughout the rt codebase

/// Prefix used for all virtual environment directories
pub const VENV_PREFIX: &str = "venv_";

/// Distribution installed into every environment to provide rt's Python runtime.
pub const RUNTIME_DISTRIBUTION: &str = "rt-runtime";

/// Manifest recording both completion and the execution context configuration.
pub const RUNTIME_CONFIG: &str = "rt_config.json";

/// Version of `rt` installed into the runtime manifest.
pub const RT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Requirements directory name under riot root
pub const REQUIREMENTS_DIR: &str = "requirements";

#[cfg(test)]
mod tests {
    use super::RT_VERSION;

    #[test]
    fn bundled_runtime_version_matches_rt() {
        let project: toml::Value =
            toml::from_str(include_str!("../rt/_runtime_project/pyproject.toml")).unwrap();
        assert_eq!(project["project"]["version"].as_str(), Some(RT_VERSION));
    }

    #[test]
    fn bundled_runtime_owns_startup_and_stub_assets() {
        let assets = [
            include_str!("../rt/_runtime_project/src/_rt_runtime/__init__.py"),
            include_str!("../rt/_runtime_project/src/_rt_runtime/pytest_plugin.py"),
            include_str!("../rt/_runtime_project/purelib/_rt_runtime.pth"),
            include_str!("../rt/_runtime_project/purelib/_rt_runtime.start"),
            include_str!("../rt/_runtime_project/purelib/envier-stubs/__init__.pyi"),
            include_str!("../rt/_runtime_project/purelib/envier-stubs/env.pyi"),
            include_str!("../rt/_runtime_project/purelib/envier-stubs/mypy.pyi"),
            include_str!("../rt/_runtime_project/purelib/envier-stubs/validators.pyi"),
        ];
        assert!(assets.iter().all(|contents| !contents.trim().is_empty()));
    }
}
