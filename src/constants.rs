//! Constants used throughout the rt codebase

/// Prefix used for all virtual environment directories
pub const VENV_PREFIX: &str = "venv_";

/// Marker file indicating a virtual environment has been fully built
pub const DONE_MARKER: &str = ".riot_done";

/// Requirements directory name under riot root
pub const REQUIREMENTS_DIR: &str = "requirements";

/// Development install directory name
pub const VENV_SELF_DIR: &str = "venv_self";

/// Dependencies install directory name
pub const VENV_DEPS_DIR: &str = "venv_deps";
