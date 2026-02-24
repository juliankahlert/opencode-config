use std::path::PathBuf;
use thiserror::Error;

use crate::config::ConfigError;
use crate::create_builder::CreateBuilder;
use crate::options::RunOptions;
use crate::substitute::SubstituteError;
use crate::template::TemplateError;

#[derive(Debug, Error)]
pub enum CreateError {
    #[error("config error: {0}")]
    Config(#[from] ConfigError),
    #[error("template error: {0}")]
    Template(#[from] TemplateError),
    #[error("substitution error: {0}")]
    Substitute(#[from] SubstituteError),
    #[error("invalid template name: {name} (use base names without extensions)")]
    InvalidTemplateName { name: String },
    #[error("palette not found: {name}")]
    MissingPalette { name: String },
    #[error("output already exists: {path}")]
    OutputExists { path: PathBuf },
    #[error("failed to write output at {path}")]
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
}

pub struct CreateOptions {
    pub template: String,
    pub palette: String,
    pub out: PathBuf,
    pub force: bool,
    pub run_options: RunOptions,
    pub config_dir: PathBuf,
}

pub fn run(options: CreateOptions) -> Result<(), CreateError> {
    CreateBuilder::new(options).run()
}
