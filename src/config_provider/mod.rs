#[cfg(feature = "provider-pyo3")]
mod fake_ruamel_yaml;
#[cfg(feature = "provider-pyo3")]
mod pyo3;
#[cfg(feature = "provider-rustpython")]
mod rustpython;

use std::{collections::HashMap, path::Path};

use indexmap::IndexMap;

use crate::error::RtResult;

#[cfg(feature = "provider-pyo3")]
pub use pyo3::Pyo3ConfigProvider;
#[cfg(feature = "provider-rustpython")]
#[allow(unused_imports)]
pub use rustpython::RustPythonConfigProvider;

pub type ProviderServices = HashMap<String, Vec<String>>;

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize)]
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
    pub services: Option<ProviderServices>,
}

pub trait ConfigProvider {
    fn load(riotfile_path: &Path) -> RtResult<LoadedConfig>;
}
