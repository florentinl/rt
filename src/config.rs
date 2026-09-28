use std::path::PathBuf;

pub struct RepoConfig {
    pub project_root: PathBuf,
    pub cache_root: PathBuf,
}

pub enum Selector {
    Pattern(String),
    Generic {
        python: Option<Vec<String>>,
        pattern: Option<String>,
        test: Option<String>,
    },
}
