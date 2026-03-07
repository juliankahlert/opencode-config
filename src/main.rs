use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::Parser;
use opencode_config::cli::{
    Cli, Commands, ConflictStrategy, PaletteCommands, PaletteFormat, PaletteMerge, RenderFormat,
    SchemaCommands, ValidateFormat,
};
use opencode_config::options::{resolve_env_flag_sources, resolve_run_options};
use opencode_config::{
    completions, compose, config, create, decompose, diff, palette_io, render, schema, template,
    validate, wizard,
};

fn resolve_env_allow(cli: &Cli) -> Option<bool> {
    if cli.no_env {
        Some(false)
    } else {
        cli.env_allow
    }
}

fn resolve_env_mask_logs(cli: &Cli) -> Option<bool> {
    if cli.no_env_mask_logs {
        Some(false)
    } else {
        cli.env_mask_logs
    }
}

/// Print a unified diff of the rendered preview against the existing output
/// file (or `/dev/null` when the file does not exist yet).
///
/// Exits with code 1 when changes are detected (mirroring `diff(1)` semantics)
/// and returns normally (exit 0) when the output would be identical.
fn handle_dry_run_diff(out_path: &std::path::Path, preview: &str) -> anyhow::Result<()> {
    let display_path = out_path.display().to_string();
    let (old_label, old_content) = if out_path.exists() {
        let content = std::fs::read_to_string(out_path)
            .with_context(|| format!("failed to read existing {display_path}"))?;
        (format!("a/{display_path}"), content)
    } else {
        ("/dev/null".to_string(), String::new())
    };
    let new_label = format!("b/{display_path}");
    let diff_text = diff::format_diff(&old_label, &old_content, &new_label, preview);
    if diff_text.is_empty() {
        println!("[DRY-RUN] No changes to {display_path}");
    } else {
        print!("{diff_text}");
        std::process::exit(1);
    }
    Ok(())
}

/// Resolve the compose `input` argument to a fragment directory.
///
/// Returns `(resolved_dir, template_name)` where `template_name` is
/// `Some(name)` when the input was resolved via template-name lookup.
///
/// For plain names (`is_valid_template_name`), template-name resolution
/// takes priority over a same-named local directory in the CWD.  This
/// ensures `compose default` selects the config-dir template even when a
/// local `default/` directory exists.
fn resolve_compose_input(
    input: &str,
    config_dir: &Path,
) -> Result<(PathBuf, Option<String>), compose::ComposeError> {
    let literal = PathBuf::from(input);

    // 1) If input looks like a template name, prefer template-name resolution.
    //    Compose always targets directories, so check for `<name>.d/` directly
    //    to avoid ambiguity errors when both `<name>.json` and `<name>.d/`
    //    coexist (which is the normal state after a successful compose).
    if template::is_valid_template_name(input) {
        let frag_dir = config_dir.join("template.d").join(format!("{input}.d"));
        if frag_dir.is_dir() {
            return Ok((frag_dir, Some(input.to_string())));
        }

        // No `.d/` directory — fall back to resolve_template_source for
        // file-only error reporting (NotAFragmentDir).
        let source = template::resolve_template_source(config_dir, input)?;
        match source {
            template::TemplateSource::Directory(dir) => {
                return Ok((dir, Some(input.to_string())));
            }
            template::TemplateSource::File(ref path) if path.exists() => {
                return Err(compose::ComposeError::NotAFragmentDir {
                    name: input.to_string(),
                });
            }
            // Fallback path from resolve_template_source does not exist —
            // template was not found; fall through to literal-path treatment.
            template::TemplateSource::File(_) => {}
        }
    }

    // 2) Literal directory fallback (local dir in CWD, or explicit path).
    if literal.is_dir() {
        return Ok((literal, None));
    }

    // 3) Fall back to literal path (compose will report InputDirNotFound).
    Ok((literal, None))
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    {
        use tracing_subscriber::EnvFilter;

        let default_level = if cli.verbose { "debug" } else { "warn" };
        let filter =
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .init();
    }

    match &cli.command {
        Commands::Create(args) => {
            let config_dir = config::resolve_config_dir(cli.config.as_ref().cloned())?;
            let cli_strict = if cli.no_strict {
                Some(false)
            } else {
                cli.strict
            };
            let run_options = resolve_run_options(
                cli_strict,
                resolve_env_allow(&cli),
                resolve_env_mask_logs(&cli),
                &config_dir,
            )?;
            if args.interactive {
                let wizard_options = wizard::WizardOptions {
                    template: args.template.clone(),
                    palette: args.palette.clone(),
                    out: args.out.clone(),
                    force: args.force,
                    run_options,
                    config_dir,
                };
                wizard::run(wizard_options).context("create wizard failed")?;
            } else {
                let template = args
                    .template
                    .clone()
                    .context("template argument is required unless --interactive")?;
                let palette = args
                    .palette
                    .clone()
                    .context("palette argument is required unless --interactive")?;
                let options = create::CreateOptions {
                    template,
                    palette,
                    out: args.out.clone(),
                    force: args.force,
                    dry_run: args.dry_run,
                    run_options,
                    config_dir,
                };
                if args.dry_run {
                    let preview = create::run_preview(options).context("create dry-run failed")?;
                    handle_dry_run_diff(&args.out, &preview)?;
                } else {
                    create::run(options).context("create command failed")?;
                }
            }
        }
        Commands::Switch(args) => {
            let config_dir = config::resolve_config_dir(cli.config.as_ref().cloned())?;
            let cli_strict = if cli.no_strict {
                Some(false)
            } else {
                cli.strict
            };
            let run_options = resolve_run_options(
                cli_strict,
                resolve_env_allow(&cli),
                resolve_env_mask_logs(&cli),
                &config_dir,
            )?;
            let options = create::CreateOptions {
                template: args.template.clone(),
                palette: args.palette.clone(),
                out: args.out.clone(),
                // Map Commands::Switch to CreateOptions with force = true
                // so switch implicitly overwrites the output file.
                force: true,
                dry_run: args.dry_run,
                run_options,
                config_dir,
            };
            if args.dry_run {
                let preview = create::run_preview(options).context("switch dry-run failed")?;
                handle_dry_run_diff(&args.out, &preview)?;
            } else {
                create::run(options).context("switch command failed")?;
            }
        }
        Commands::ListTemplates => {
            let config_dir = config::resolve_config_dir(cli.config.as_ref().cloned())?;
            let mut names = template::list_templates(&config_dir)?;
            names.sort();
            for name in names {
                println!("{name}");
            }
        }
        Commands::ListPalettes => {
            let config_dir = config::resolve_config_dir(cli.config.as_ref().cloned())?;
            let configs = config::load_model_configs(&config_dir)?;
            let mut names: Vec<String> = configs.palettes.keys().cloned().collect();
            names.sort();
            for name in names {
                println!("{name}");
            }
        }
        Commands::Completions(args) => {
            completions::generate(args.shell, &args.out_dir)?;
        }
        Commands::Validate(args) => {
            let config_dir = config::resolve_config_dir(cli.config.as_ref().cloned())?;
            let cli_strict = if cli.no_strict {
                Some(false)
            } else {
                cli.strict
            };
            let run_options = resolve_run_options(
                cli_strict,
                resolve_env_allow(&cli),
                resolve_env_mask_logs(&cli),
                &config_dir,
            )?;
            let (env_allow, env_mask_logs) = resolve_env_flag_sources(
                resolve_env_allow(&cli),
                resolve_env_mask_logs(&cli),
                &config_dir,
            )?;
            let opts = validate::ValidateOpts {
                templates: args.templates.clone(),
                palettes_path: args.palettes.clone(),
                strict: run_options.strict,
                env_allow,
                env_mask_logs,
                schema: args.schema,
            };
            let report =
                validate::validate_dir(&config_dir, opts).context("validate command failed")?;
            match args.format {
                ValidateFormat::Text => {
                    let text = validate::format_report_text(&report);
                    println!("{text}");
                }
                ValidateFormat::Json => {
                    let json_report = validate::format_report_json(&report);
                    let data = serde_json::to_string_pretty(&json_report)
                        .context("failed to serialize validation report")?;
                    println!("{data}");
                }
            }
            if report.counts.errors > 0 {
                std::process::exit(1);
            }
        }
        Commands::Render(args) => {
            let config_dir = config::resolve_config_dir(cli.config.as_ref().cloned())?;
            let cli_strict = if cli.no_strict {
                Some(false)
            } else {
                cli.strict
            };
            let run_options = resolve_run_options(
                cli_strict,
                resolve_env_allow(&cli),
                resolve_env_mask_logs(&cli),
                &config_dir,
            )?;
            let format = match args.format {
                RenderFormat::Json => render::OutputFormat::Json,
                RenderFormat::Yaml => render::OutputFormat::Yaml,
            };
            let output = render::render(render::RenderOptions {
                template: args.template.clone(),
                palette: args.palette.clone(),
                format,
                strict: run_options.strict,
                env_allow: run_options.env_allow,
                env_mask_logs: run_options.env_mask_logs,
                config_dir,
            })
            .context("render command failed")?;

            if args.dry_run {
                if args.out == "-" {
                    println!("{data}", data = output.data);
                } else {
                    let path = &args.out;
                    let target = std::path::Path::new(path);
                    let (old_label, old_content) = if target.exists() {
                        let content = std::fs::read_to_string(target)
                            .with_context(|| format!("failed to read existing {path}"))?;
                        (format!("a/{path}"), content)
                    } else {
                        ("/dev/null".to_string(), String::new())
                    };
                    let new_label = format!("b/{path}");
                    let diff_text =
                        diff::format_diff(&old_label, &old_content, &new_label, &output.data);
                    if diff_text.is_empty() {
                        println!("[DRY-RUN] No changes to {path}");
                    } else {
                        print!("{diff_text}");
                        std::process::exit(1);
                    }
                }
                return Ok(());
            }

            if args.out == "-" {
                println!("{data}", data = output.data);
            } else {
                std::fs::write(&args.out, output.data)
                    .with_context(|| format!("failed to write render output to {}", args.out))?;
            }
        }
        Commands::Schema(args) => match &args.command {
            SchemaCommands::Generate(generate) => {
                let config_dir = config::resolve_config_dir(cli.config.as_ref().cloned())?;
                let out_dir = generate.out.clone().unwrap_or_else(|| {
                    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
                });
                let path = schema::generate_schema_file(schema::SchemaGenerateOptions {
                    palette: Some(generate.palette.clone()),
                    out_dir,
                    config_dir,
                })
                .context("failed to generate schema")?;
                println!("{path}", path = path.display());
            }
        },
        Commands::Palette(args) => match &args.command {
            PaletteCommands::Export(export) => {
                let config_dir = config::resolve_config_dir(cli.config.as_ref().cloned())?;
                let format = match export.format {
                    PaletteFormat::Json => palette_io::PaletteFormat::Json,
                    PaletteFormat::Yaml => palette_io::PaletteFormat::Yaml,
                };
                let output = palette_io::export_palette(palette_io::ExportOptions {
                    name: export.name.clone(),
                    format,
                    config_dir,
                })
                .context("palette export failed")?;
                if export.out == "-" {
                    println!("{data}", data = output.data);
                } else {
                    let out_path = PathBuf::from(&export.out);
                    if out_path.exists() && !export.force {
                        anyhow::bail!("output already exists: {path}", path = export.out);
                    }
                    std::fs::write(&out_path, output.data).with_context(|| {
                        format!("failed to write palette export to {}", export.out)
                    })?;
                }
            }
            PaletteCommands::Import(import) => {
                let config_dir = config::resolve_config_dir(cli.config.as_ref().cloned())?;
                let merge = match import.merge {
                    PaletteMerge::Abort => palette_io::MergeStrategy::Abort,
                    PaletteMerge::Overwrite => palette_io::MergeStrategy::Overwrite,
                    PaletteMerge::Merge => palette_io::MergeStrategy::Merge,
                };
                let report = palette_io::import_palette(palette_io::ImportOptions {
                    from: import.from.clone(),
                    name: import.name.clone(),
                    merge,
                    dry_run: import.dry_run,
                    force: import.force,
                    config_dir,
                })
                .context("palette import failed")?;
                let prefix = match report.status {
                    palette_io::ImportStatus::Applied => "[APPLIED]",
                    palette_io::ImportStatus::DryRun => "[DRY-RUN]",
                    palette_io::ImportStatus::NeedsForce => "[NO-WRITE]",
                    palette_io::ImportStatus::Aborted => "[ABORTED]",
                };
                let verb = match report.status {
                    palette_io::ImportStatus::Applied => "Applied",
                    palette_io::ImportStatus::Aborted => "Aborted",
                    palette_io::ImportStatus::DryRun | palette_io::ImportStatus::NeedsForce => {
                        "Would"
                    }
                };
                if report.created {
                    println!(
                        "{prefix} {verb} add palette '{name}'",
                        name = report.palette_name
                    );
                } else {
                    println!(
                        "{prefix} {verb} update palette '{name}'",
                        name = report.palette_name
                    );
                }
                if report.conflicts.is_empty() {
                    println!("{prefix} No conflicts detected");
                } else {
                    println!(
                        "{prefix} Conflicts detected ({count})",
                        count = report.conflicts.len()
                    );
                    for conflict in &report.conflicts {
                        println!("{prefix} - {conflict}");
                    }
                }

                if report.status == palette_io::ImportStatus::NeedsForce {
                    println!("{prefix} Run with --force to apply changes");
                }

                if report.status == palette_io::ImportStatus::Aborted {
                    anyhow::bail!(
                        "palette import aborted; resolve conflicts or choose another merge strategy"
                    );
                }
            }
        },
        Commands::Decompose(args) => {
            let config_dir = config::resolve_config_dir(cli.config.as_ref().cloned())?;
            let options = decompose::DecomposeOptions {
                template: args.template.clone(),
                config_dir,
                dry_run: args.dry_run,
                verify: args.verify,
                force: args.force,
            };
            if args.dry_run {
                let preview =
                    decompose::run_preview(options).context("decompose dry-run failed")?;
                print!("{preview}");
            } else {
                decompose::run(options).context("decompose command failed")?;
            }
        }
        Commands::Compose(args) => {
            let config_dir = config::resolve_config_dir(cli.config.as_ref().cloned())?;
            let (input_dir, template_name) = resolve_compose_input(&args.input, &config_dir)?;
            let out_path = if let Some(ref out) = args.out {
                out.clone()
            } else if let Some(ref name) = template_name {
                config_dir.join("template.d").join(format!("{name}.json"))
            } else {
                PathBuf::from("opencode.json")
            };
            let cli_strict = if cli.no_strict {
                Some(false)
            } else {
                cli.strict
            };
            let run_options = resolve_run_options(
                cli_strict,
                resolve_env_allow(&cli),
                resolve_env_mask_logs(&cli),
                &config_dir,
            )?;
            let conflict = match args.conflict {
                ConflictStrategy::Error => compose::Conflict::Error,
                ConflictStrategy::LastWins => compose::Conflict::LastWins,
                ConflictStrategy::Interactive => compose::Conflict::Interactive,
            };
            let pretty = match (args.pretty, args.minify) {
                (true, _) => true,
                (_, true) => false,
                // Default: pretty output when neither flag is provided
                (false, false) => true,
            };
            let options = compose::ComposeOptions {
                input_dir,
                out: out_path.clone(),
                dry_run: args.dry_run,
                backup: args.backup,
                pretty,
                verify: args.verify,
                force: args.force,
                conflict,
                run_options,
                config_dir,
            };
            if args.dry_run {
                let preview = compose::run_preview(options).context("compose dry-run failed")?;
                handle_dry_run_diff(&out_path, &preview)?;
            } else {
                compose::run(options).context("compose command failed")?;
            }
        }
    }

    Ok(())
}
