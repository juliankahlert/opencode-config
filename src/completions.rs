use std::fs;
use std::path::Path;

use clap::CommandFactory;

use crate::cli::{Cli, Shell};

pub fn generate(shell: Shell, out_dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(out_dir)?;
    let mut command = Cli::command();
    clap_complete::generate_to(shell.to_clap(), &mut command, "opencode-config", out_dir)?;
    Ok(())
}
