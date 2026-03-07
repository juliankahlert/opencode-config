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

    /// Enable verbose (debug-level) logging
    #[arg(short, long, global = true)]
    pub verbose: bool,
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
    /// Decompose opencode.json into per-section fragment files
    Decompose(DecomposeArgs),
    /// Compose fragment files back into a single opencode.json
    Compose(ComposeArgs),
}

#[derive(Parser, Debug)]
pub struct CreateArgs {
    /// Template name to use for config generation.
    ///
    /// Accepts a template name resolved under `template.d/`: the tool looks
    /// for `<name>.json|yaml|yml` first, then for the fragment directory
    /// `<name>.d/`. If both a file and a directory match, resolution fails
    /// with an ambiguity error.
    ///
    /// Directory templates (`<name>.d/`) are assembled by merging fragments
    /// in lexicographic order before rendering. When combined with `--dry-run`
    /// or `--force`, the merged result is what gets previewed or written.
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

    /// Preview output without writing files
    #[arg(long, conflicts_with = "interactive")]
    pub dry_run: bool,

    /// Run the interactive create wizard
    #[arg(short = 'i', long = "interactive")]
    pub interactive: bool,
}

#[derive(Parser, Debug)]
pub struct SwitchArgs {
    /// Switch behaves like `create` but implicitly overwrites the output file.
    ///
    /// Template name to use, resolved under `template.d/`: the tool looks
    /// for `<name>.json|yaml|yml` first, then for the fragment directory
    /// `<name>.d/`. If both a file and a directory match, resolution fails
    /// with an ambiguity error.
    ///
    /// Directory templates (`<name>.d/`) are assembled by merging fragments
    /// in lexicographic order before rendering. With `--dry-run` the merged
    /// result is previewed without writing; otherwise the output file is
    /// overwritten unconditionally.
    pub template: String,
    /// Palette name to use
    pub palette: String,

    /// Output file path
    #[arg(short = 'o', long, default_value = "opencode.json")]
    pub out: PathBuf,

    /// Preview output without writing files
    #[arg(long)]
    pub dry_run: bool,
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
    /// Glob pattern(s) selecting templates to validate.
    ///
    /// Defaults to scanning `template.d/` for all `*.json`, `*.yaml`, and
    /// `*.yml` files as well as fragment directories (`*.d/`). Each pattern
    /// may match single-file templates or fragment directories; both kinds
    /// are validated.
    ///
    /// Fragment directories are assembled by merging their contents in
    /// lexicographic order before validation. During default discovery
    /// (no patterns given), a name that resolves to both a file and a
    /// directory is reported as an ambiguity error.
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
    /// Template name or path to use for rendering.
    ///
    /// Accepts either a **name** or a **file path**. Values containing a
    /// path separator (`/` or `\`) or a filename extension (e.g. `.json`)
    /// are treated as literal filesystem paths. Plain names are resolved
    /// under `template.d/`: the tool looks for `<name>.json|yaml|yml`
    /// first, then for the fragment directory `<name>.d/`. If both a file
    /// and a directory match, resolution fails with an ambiguity error.
    ///
    /// Directory templates (`<name>.d/`) are assembled by merging fragments
    /// in lexicographic order before rendering.
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

    /// Preview without writing to a file.
    ///
    /// For file targets, shows a unified diff against the existing content.
    /// For stdout (`-`), prints the rendered output directly.
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

#[derive(Parser, Debug)]
pub struct DecomposeArgs {
    /// Template name to decompose (must be a single file, not already a directory)
    pub template: String,

    /// Preview decomposition without writing files
    #[arg(long)]
    pub dry_run: bool,

    /// Verify roundtrip: reassemble fragments and compare with original
    #[arg(long)]
    pub verify: bool,

    /// Overwrite target directory if it already exists
    #[arg(long)]
    pub force: bool,
}

#[derive(Parser, Debug)]
pub struct ComposeArgs {
    /// Template name or directory path to compose.
    ///
    /// Accepts either a **template name** resolved under `template.d/`
    /// (e.g. `default`) or a **literal directory path** containing fragment
    /// files (e.g. `./fragments`). Values containing a path separator
    /// (`/` or `\`) are treated as filesystem paths; plain names are
    /// resolved as template names first, falling back to a local directory.
    #[arg(default_value = ".")]
    pub input: String,

    /// Output file path.
    ///
    /// When omitted, output is derived automatically:
    /// - **template-name** input → `<config_dir>/template.d/<name>.json`
    /// - **literal directory** input → `opencode.json` in the current directory
    #[arg(short = 'o', long)]
    pub out: Option<PathBuf>,

    /// Preview output without writing files
    #[arg(long)]
    pub dry_run: bool,

    /// Create a backup of the output file before overwriting
    #[arg(long)]
    pub backup: bool,

    /// Pretty-print JSON output (default when neither flag is given)
    #[arg(long, conflicts_with = "minify")]
    pub pretty: bool,

    /// Minify JSON output
    #[arg(long, conflicts_with = "pretty")]
    pub minify: bool,

    /// Verify round-trip fidelity after compose
    #[arg(long)]
    pub verify: bool,

    /// Overwrite output if it exists
    #[arg(long)]
    pub force: bool,

    /// Strategy for handling conflicting keys across fragments
    #[arg(long, value_enum, default_value_t = ConflictStrategy::Error)]
    pub conflict: ConflictStrategy,
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

/// Strategy for resolving key conflicts during compose
#[derive(ValueEnum, Copy, Clone, Debug, Eq, PartialEq)]
pub enum ConflictStrategy {
    /// Abort on conflicting keys (default)
    Error,
    /// Last fragment wins on conflict
    LastWins,
    /// Prompt the user to resolve each conflict interactively
    Interactive,
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
