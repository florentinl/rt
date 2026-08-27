//! Constants used throughout the rt codebase

/// Prefix used for all virtual environment directories
pub const VENV_PREFIX: &str = "venv_";

/// Marker file recording which `rt` version last configured a virtual environment.
pub const DONE_MARKER: &str = ".riot_done";

/// Version of `rt` stamped into `DONE_MARKER`.
pub const RT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Requirements directory name under riot root
pub const REQUIREMENTS_DIR: &str = "requirements";
