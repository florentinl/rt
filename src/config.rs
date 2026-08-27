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

pub struct RunConfig {
    pub command_override: Option<String>,
    pub cmdargs: Vec<String>,
    pub action_label: String,
}

impl RepoConfig {
    #[must_use]
    pub const fn new(riotfile_path: PathBuf, riot_root: PathBuf) -> Self {
        Self {
            riotfile_path,
            riot_root,
        }
    }
}
