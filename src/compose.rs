//! Compose fragmented template directories back into a single monolithic file.

use std::path::PathBuf;

use thiserror::Error;

use crate::config::ConfigError;
use crate::options::RunOptions;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conflict {
    Error,
    LastWins,
    Interactive,
}

#[derive(Debug, Error)]
pub enum ComposeError {
    #[error("config error: {0}")]
    Config(#[from] ConfigError),
    #[error("failed to read {path}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write {path}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to serialize output")]
    Serialize {
        #[source]
        source: serde_json::Error,
    },
    #[error("conflicting keys detected: {keys:?}")]
    ConflictDetected { keys: Vec<String> },
    #[error("schema violation in composed output")]
    SchemaViolation,
}

pub struct ComposeOptions {
    pub input_dir: PathBuf,
    pub out: PathBuf,
    pub dry_run: bool,
    pub backup: bool,
    pub pretty: bool,
    pub verify: bool,
    pub force: bool,
    pub conflict: Conflict,
    pub run_options: RunOptions,
    pub config_dir: PathBuf,
}

pub fn run(_options: ComposeOptions) -> Result<(), ComposeError> {
    todo!("compose::run")
}

pub fn run_preview(_options: ComposeOptions) -> Result<String, ComposeError> {
    todo!("compose::run_preview")
}
