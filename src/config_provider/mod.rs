mod fake_ruamel_yaml;
mod pyo3;

use std::{collections::HashMap, path::Path};

use indexmap::IndexMap;

use crate::error::RtResult;

pub use pyo3::Pyo3ConfigProvider;

pub type ProviderServices = HashMap<String, Vec<String>>;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
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
