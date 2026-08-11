//! Constants used throughout the rt codebase

/// Prefix used for all virtual environment directories
pub const VENV_PREFIX: &str = "venv_";

/// Marker file indicating a virtual environment has been fully built.
///
/// The file records the `rt` version that produced the directory, so a build
/// can only be reused by the exact same version (see `RT_VERSION`).
pub const DONE_MARKER: &str = ".riot_done";

/// Version of `rt` stamped into `DONE_MARKER`.
///
/// Anything cached by a different version is rebuilt: the layout of the
/// generated venvs (`sitecustomize.py`, `riot.pth`, requirements, uv flags) is
/// an implementation detail of a given `rt` release, so a stamp mismatch means
/// the cached directory cannot be trusted.
pub const RT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Requirements directory name under riot root
pub const REQUIREMENTS_DIR: &str = "requirements";

/// Development install directory name
pub const VENV_SELF_DIR: &str = "venv_self";

/// Dependencies install directory name
pub const VENV_DEPS_DIR: &str = "venv_deps";
