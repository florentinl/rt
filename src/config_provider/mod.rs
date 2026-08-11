mod pyo3;

use std::{collections::HashMap, path::Path};

use indexmap::IndexMap;

use crate::error::RtResult;

pub use pyo3::PyVenv;
pub use pyo3::Pyo3ConfigProvider;

/// Suitespec metadata attached to a suite pattern.
///
/// Mirrors the fields `scripts/run-tests` reads out of `tests/suitespec.py`:
/// `services` (plus `testagent` when `snapshot` is set) and `env`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProviderSuite {
    pub services: Vec<String>,
    pub env: IndexMap<String, String>,
}

/// Map of suitespec `pattern` -> suite metadata.
pub type ProviderSuites = HashMap<String, ProviderSuite>;

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct ProviderVenvNode {
    pub name: Option<String>,
    pub command: Option<String>,
    pub pys: Vec<String>,
    pub pkgs: IndexMap<String, Vec<String>>,
    pub env: IndexMap<String, Vec<String>>,
    pub create: Option<bool>,
    pub skip_dev_install: Option<bool>,
    pub venvs: Vec<Self>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LoadedConfig {
    pub root: ProviderVenvNode,
    pub suites: Option<ProviderSuites>,
}

pub trait ConfigProvider {
    fn load(riotfile_path: &Path) -> RtResult<LoadedConfig>;
}
