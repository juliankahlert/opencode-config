use std::path::PathBuf;

use anyhow::Context;
use clap::Parser;
use opencode_config::cli::{
    Cli, Commands, PaletteCommands, PaletteFormat, PaletteMerge, RenderFormat, SchemaCommands,
    ValidateFormat,
};
use opencode_config::options::{resolve_env_flag_sources, resolve_run_options};
use opencode_config::{
    completions, config, create, palette_io, render, schema, template, validate, wizard,
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

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

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
                    run_options,
                    config_dir,
                };
                create::run(options).context("create command failed")?;
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
                run_options,
                config_dir,
            };
            create::run(options).context("switch command failed")?;
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
                    println!(
                        "[DRY-RUN] Would write {lines} lines to {path}",
                        lines = output.lines,
                        path = args.out
                    );
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
    }

    Ok(())
}
