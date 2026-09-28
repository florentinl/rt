//! Constants used throughout the rt codebase

pub const ENVIRONMENTS_DIR: &str = "environments";
pub const PROFILES_DIR: &str = "profiles";
pub const UV_PROJECTS_DIR: &str = "uv-projects";

/// Distribution installed into every environment to provide rt's Python runtime.
pub const RUNTIME_DISTRIBUTION: &str = "rt-runtime";

pub const RUNTIME_CONFIG: &str = "rt-context.json";

/// Version of `rt` installed into the runtime manifest.
pub const RT_VERSION: &str = env!("CARGO_PKG_VERSION");

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
