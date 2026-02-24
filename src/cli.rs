use std::path::PathBuf;

use clap::{ArgAction, Parser, Subcommand, ValueEnum};

#[derive(Parser, Debug)]
#[command(name = "opencode-config", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Enable strict mode (overrides config and environment)
    #[arg(
        short = 'S',
        long,
        global = true,
        num_args = 0..=1,
        default_missing_value = "true",
        value_parser = clap::builder::BoolishValueParser::new()
    )]
    pub strict: Option<bool>,

    /// Disable strict mode (overrides config and environment)
    #[arg(
        long = "no-strict",
        global = true,
        action = ArgAction::SetTrue,
        conflicts_with = "strict"
    )]
    pub no_strict: bool,

    /// Path to the config directory (defaults to XDG config home)
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,

    /// Allow env placeholders (overrides config)
    #[arg(
        long = "env-allow",
        global = true,
        num_args = 0..=1,
        default_missing_value = "true",
        value_parser = clap::builder::BoolishValueParser::new()
    )]
    pub env_allow: Option<bool>,

    /// Disable env placeholders (overrides config)
    #[arg(
        long = "no-env",
        global = true,
        action = ArgAction::SetTrue,
        conflicts_with = "env_allow"
    )]
    pub no_env: bool,

    /// Mask env values in logs (overrides config)
    #[arg(
        long = "env-mask-logs",
        global = true,
        num_args = 0..=1,
        default_missing_value = "true",
        value_parser = clap::builder::BoolishValueParser::new()
    )]
    pub env_mask_logs: Option<bool>,

    /// Disable masking env values in logs (overrides config)
    #[arg(
        long = "no-env-mask-logs",
        global = true,
        action = ArgAction::SetTrue,
        conflicts_with = "env_mask_logs"
    )]
    pub no_env_mask_logs: bool,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Create opencode.json from a template and palette
    Create(CreateArgs),
    /// Switch an existing opencode.json to a different palette/template
    ///
    /// The `switch` subcommand behaves like `create` but implicitly
    /// overwrites the destination file (it sets force = true).
    Switch(SwitchArgs),
    /// List available templates
    ListTemplates,
    /// List available palettes
    ListPalettes,
    /// Generate shell completions
    Completions(CompletionsArgs),
    /// Validate templates and palettes
    Validate(ValidateArgs),
    /// Render a template and palette without writing config
    Render(RenderArgs),
    /// Generate JSON Schema artifacts
    Schema(SchemaArgs),
    /// Import or export palettes
    Palette(PaletteArgs),
}

#[derive(Parser, Debug)]
pub struct CreateArgs {
    /// Template name to use
    #[arg(required_unless_present = "interactive")]
    pub template: Option<String>,
    /// Palette name to use
    #[arg(required_unless_present = "interactive")]
    pub palette: Option<String>,

    /// Output file path
    #[arg(short = 'o', long, default_value = "opencode.json")]
    pub out: PathBuf,

    /// Overwrite output if it exists
    #[arg(long)]
    pub force: bool,

    /// Run the interactive create wizard
    #[arg(short = 'i', long = "interactive")]
    pub interactive: bool,
}

#[derive(Parser, Debug)]
pub struct SwitchArgs {
    /// Switch behaves like `create` but implicitly overwrites the output file.
    /// Template name to use
    pub template: String,
    /// Palette name to use
    pub palette: String,

    /// Output file path
    #[arg(short = 'o', long, default_value = "opencode.json")]
    pub out: PathBuf,
}

#[derive(Parser, Debug)]
pub struct CompletionsArgs {
    /// Shell to generate completions for
    #[arg(value_enum)]
    pub shell: Shell,

    /// Output directory
    #[arg(long)]
    pub out_dir: PathBuf,
}

#[derive(Parser, Debug)]
pub struct ValidateArgs {
    /// Template glob(s) (defaults to template.d/*.json|yaml|yml)
    #[arg(long = "templates", value_name = "GLOB")]
    pub templates: Vec<String>,

    /// Palettes file override (defaults to model-configs.yaml)
    #[arg(long = "palettes", value_name = "FILE")]
    pub palettes: Option<PathBuf>,

    /// Output format
    #[arg(long, value_enum, default_value_t = ValidateFormat::Text)]
    pub format: ValidateFormat,

    /// Validate rendered output against JSON Schema
    #[arg(long)]
    pub schema: bool,
}

#[derive(Parser, Debug)]
pub struct RenderArgs {
    /// Template name or path to use
    #[arg(short = 't', long = "template")]
    pub template: String,
    /// Palette name to use
    #[arg(short = 'p', long = "palette")]
    pub palette: String,

    /// Output file path (use '-' for stdout)
    #[arg(short = 'o', long, default_value = "-")]
    pub out: String,

    /// Output format
    #[arg(long, value_enum, default_value_t = RenderFormat::Json)]
    pub format: RenderFormat,

    /// Print without writing output
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Parser, Debug)]
pub struct PaletteArgs {
    #[command(subcommand)]
    pub command: PaletteCommands,
}

#[derive(Subcommand, Debug)]
pub enum PaletteCommands {
    /// Export a palette to a file or stdout
    Export(PaletteExportArgs),
    /// Import a palette from a file
    Import(PaletteImportArgs),
}

#[derive(Parser, Debug)]
pub struct PaletteExportArgs {
    /// Palette name to export
    #[arg(long = "name", value_name = "PALETTE")]
    pub name: String,

    /// Output file path (use '-' for stdout)
    #[arg(short = 'o', long, default_value = "-")]
    pub out: String,

    /// Output format
    #[arg(long, value_enum, default_value_t = PaletteFormat::Yaml)]
    pub format: PaletteFormat,

    /// Overwrite output if it exists
    #[arg(long)]
    pub force: bool,
}

#[derive(Parser, Debug)]
pub struct PaletteImportArgs {
    /// File path to import
    #[arg(long = "from", value_name = "FILE")]
    pub from: PathBuf,

    /// Override palette name (defaults to file stem)
    #[arg(long = "name", value_name = "PALETTE")]
    pub name: Option<String>,

    /// Merge strategy
    #[arg(long, value_enum, default_value_t = PaletteMerge::Abort)]
    pub merge: PaletteMerge,

    /// Print without writing output
    #[arg(long)]
    pub dry_run: bool,

    /// Persist changes to model-configs.yaml
    #[arg(long)]
    pub force: bool,
}

#[derive(Parser, Debug)]
pub struct SchemaArgs {
    #[command(subcommand)]
    pub command: SchemaCommands,
}

#[derive(Subcommand, Debug)]
pub enum SchemaCommands {
    /// Generate a schema for a palette
    Generate(SchemaGenerateArgs),
}

#[derive(Parser, Debug)]
pub struct SchemaGenerateArgs {
    /// Palette name to generate schema for
    #[arg(long, value_name = "PALETTE")]
    pub palette: String,

    /// Output directory (defaults to current directory)
    #[arg(long, value_name = "DIR")]
    pub out: Option<PathBuf>,
}

#[derive(ValueEnum, Copy, Clone, Debug, Eq, PartialEq)]
pub enum ValidateFormat {
    Text,
    Json,
}

#[derive(ValueEnum, Copy, Clone, Debug, Eq, PartialEq)]
pub enum RenderFormat {
    Json,
    Yaml,
}

#[derive(ValueEnum, Copy, Clone, Debug, Eq, PartialEq)]
pub enum PaletteFormat {
    Json,
    Yaml,
}

#[derive(ValueEnum, Copy, Clone, Debug, Eq, PartialEq)]
pub enum PaletteMerge {
    Abort,
    Overwrite,
    Merge,
}

#[derive(ValueEnum, Copy, Clone, Debug, Eq, PartialEq)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
    Elvish,
    PowerShell,
}

impl Shell {
    pub fn to_clap(self) -> clap_complete::Shell {
        match self {
            Shell::Bash => clap_complete::Shell::Bash,
            Shell::Zsh => clap_complete::Shell::Zsh,
            Shell::Fish => clap_complete::Shell::Fish,
            Shell::Elvish => clap_complete::Shell::Elvish,
            Shell::PowerShell => clap_complete::Shell::PowerShell,
        }
    }
}
