use std::path::PathBuf;

pub struct RepoConfig {
    pub riotfile_path: PathBuf,
    pub riot_root: PathBuf,
}

pub enum Selector {
    Pattern(String),
    Generic {
        python: Option<Vec<String>>,
        pattern: Option<String>,
        test: Option<String>,
    },
}
