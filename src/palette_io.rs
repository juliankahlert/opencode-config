use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::config::{ConfigError, ModelConfigs, Palette, load_model_configs};

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum PaletteFormat {
    Json,
    Yaml,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum MergeStrategy {
    Abort,
    Overwrite,
    Merge,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum ImportStatus {
    Applied,
    DryRun,
    NeedsForce,
    Aborted,
}

#[derive(Debug, Error)]
pub enum PaletteIoError {
    #[error("config error: {0}")]
    Config(#[from] ConfigError),
    #[error("palette not found: {name}")]
    MissingPalette { name: String },
    #[error("missing palette name for import from {path}")]
    MissingImportName { path: PathBuf },
    #[error("failed to read palette at {path}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse palette at {path}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to parse yaml palette at {path}")]
    ParseYaml {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },
    #[error("unsupported palette extension at {path}: {extension}")]
    UnsupportedExtension { path: PathBuf, extension: String },
    #[error("failed to serialize palette")]
    SerializeJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to serialize palette to yaml")]
    SerializeYaml {
        #[source]
        source: serde_yaml::Error,
    },
    #[error("failed to write palettes at {path}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

pub struct ExportOptions {
    pub name: String,
    pub format: PaletteFormat,
    pub config_dir: PathBuf,
}

pub struct ExportOutput {
    pub data: String,
    pub lines: usize,
}

pub struct ImportOptions {
    pub from: PathBuf,
    pub name: Option<String>,
    pub merge: MergeStrategy,
    pub dry_run: bool,
    pub force: bool,
    pub config_dir: PathBuf,
}

pub struct ImportReport {
    pub palette_name: String,
    pub created: bool,
    pub status: ImportStatus,
    pub conflicts: Vec<String>,
}

struct ImportStart;

struct ImportLoaded {
    palette_name: String,
    incoming: Palette,
    configs: ModelConfigs,
    existing: Option<Palette>,
    created: bool,
    conflicts: Vec<String>,
}

struct ImportPlanned {
    palette_name: String,
    configs: ModelConfigs,
    created: bool,
    conflicts: Vec<String>,
    merged: Palette,
    changed: bool,
    status: ImportStatus,
}

struct PaletteImportBuilder<State> {
    options: ImportOptions,
    state: State,
}

impl PaletteImportBuilder<ImportStart> {
    fn new(options: ImportOptions) -> Self {
        Self {
            options,
            state: ImportStart,
        }
    }

    fn load(self) -> Result<PaletteImportBuilder<ImportLoaded>, PaletteIoError> {
        let palette_name = resolve_import_name(self.options.name.as_ref(), &self.options.from)?;
        let incoming = load_palette_file(&self.options.from)?;

        let configs = load_model_configs(&self.options.config_dir)?;
        let existing = configs.palettes.get(&palette_name).cloned();
        let created = existing.is_none();
        let conflicts = existing
            .as_ref()
            .map(|palette| palette_conflicts(palette, &incoming))
            .unwrap_or_default();

        Ok(PaletteImportBuilder {
            options: self.options,
            state: ImportLoaded {
                palette_name,
                incoming,
                configs,
                existing,
                created,
                conflicts,
            },
        })
    }
}

impl PaletteImportBuilder<ImportLoaded> {
    fn plan(self) -> Result<PaletteImportBuilder<ImportPlanned>, PaletteIoError> {
        let ImportLoaded {
            palette_name,
            incoming,
            configs,
            existing,
            created,
            conflicts,
        } = self.state;

        if matches!(self.options.merge, MergeStrategy::Abort) && !created && !conflicts.is_empty() {
            return Ok(PaletteImportBuilder {
                options: self.options,
                state: ImportPlanned {
                    palette_name,
                    configs,
                    created,
                    conflicts,
                    merged: incoming,
                    changed: false,
                    status: ImportStatus::Aborted,
                },
            });
        }

        let merged = match (self.options.merge, existing.as_ref()) {
            (_, None) => incoming.clone(),
            (MergeStrategy::Overwrite, Some(_)) => incoming.clone(),
            (MergeStrategy::Abort | MergeStrategy::Merge, Some(existing_palette)) => {
                merge_palette(existing_palette, &incoming)
            }
        };

        let changed = match existing.as_ref() {
            Some(existing_palette) => existing_palette != &merged,
            None => true,
        };

        let status = if self.options.dry_run {
            ImportStatus::DryRun
        } else if !self.options.force {
            if changed {
                ImportStatus::NeedsForce
            } else {
                ImportStatus::Applied
            }
        } else {
            ImportStatus::Applied
        };

        Ok(PaletteImportBuilder {
            options: self.options,
            state: ImportPlanned {
                palette_name,
                configs,
                created,
                conflicts,
                merged,
                changed,
                status,
            },
        })
    }
}

impl PaletteImportBuilder<ImportPlanned> {
    fn apply(self) -> Result<ImportReport, PaletteIoError> {
        let ImportPlanned {
            palette_name,
            mut configs,
            created,
            conflicts,
            merged,
            changed,
            status,
        } = self.state;

        if status == ImportStatus::Applied && changed {
            configs.palettes.insert(palette_name.clone(), merged);
            write_model_configs(&configs, &self.options.config_dir)?;
        }

        Ok(ImportReport {
            palette_name,
            created,
            status,
            conflicts,
        })
    }
}

pub fn export_palette(options: ExportOptions) -> Result<ExportOutput, PaletteIoError> {
    let configs = load_model_configs(&options.config_dir)?;
    let palette =
        configs
            .palettes
            .get(&options.name)
            .ok_or_else(|| PaletteIoError::MissingPalette {
                name: options.name.clone(),
            })?;

    let data = match options.format {
        PaletteFormat::Json => serde_json::to_string_pretty(palette)
            .map_err(|source| PaletteIoError::SerializeJson { source })?,
        PaletteFormat::Yaml => serde_yaml::to_string(palette)
            .map_err(|source| PaletteIoError::SerializeYaml { source })?,
    };
    let lines = data.lines().count();

    Ok(ExportOutput { data, lines })
}

pub fn import_palette(options: ImportOptions) -> Result<ImportReport, PaletteIoError> {
    PaletteImportBuilder::new(options).load()?.plan()?.apply()
}

fn load_palette_file(path: &Path) -> Result<Palette, PaletteIoError> {
    let data = fs::read_to_string(path).map_err(|source| PaletteIoError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase);

    match extension.as_deref() {
        Some("json") => serde_json::from_str(&data).map_err(|source| PaletteIoError::Parse {
            path: path.to_path_buf(),
            source,
        }),
        Some("yaml") | Some("yml") => {
            serde_yaml::from_str(&data).map_err(|source| PaletteIoError::ParseYaml {
                path: path.to_path_buf(),
                source,
            })
        }
        Some(extension) => Err(PaletteIoError::UnsupportedExtension {
            path: path.to_path_buf(),
            extension: extension.to_string(),
        }),
        None => Err(PaletteIoError::UnsupportedExtension {
            path: path.to_path_buf(),
            extension: "<none>".to_string(),
        }),
    }
}

fn resolve_import_name(name: Option<&String>, path: &Path) -> Result<String, PaletteIoError> {
    if let Some(name) = name {
        return Ok(name.clone());
    }

    path.file_stem()
        .and_then(|stem| stem.to_str())
        .map(|stem| stem.to_string())
        .filter(|stem| !stem.is_empty())
        .ok_or_else(|| PaletteIoError::MissingImportName {
            path: path.to_path_buf(),
        })
}

fn palette_conflicts(existing: &Palette, incoming: &Palette) -> Vec<String> {
    let mut conflicts = BTreeSet::new();

    for (name, agent) in &incoming.agents {
        if let Some(existing_agent) = existing.agents.get(name)
            && existing_agent != agent
        {
            conflicts.insert(format!("agent.{name}"));
        }
    }

    for (key, value) in &incoming.mapping {
        if let Some(existing_value) = existing.mapping.get(key)
            && existing_value != value
        {
            conflicts.insert(format!("mapping.{key}"));
        }
    }

    conflicts.into_iter().collect()
}

fn merge_palette(existing: &Palette, incoming: &Palette) -> Palette {
    let mut merged = existing.clone();

    for (name, agent) in &incoming.agents {
        merged
            .agents
            .entry(name.clone())
            .or_insert_with(|| agent.clone());
    }

    for (key, value) in &incoming.mapping {
        merged
            .mapping
            .entry(key.clone())
            .or_insert_with(|| value.clone());
    }

    merged
}

fn write_model_configs(configs: &ModelConfigs, config_dir: &Path) -> Result<(), PaletteIoError> {
    let data = serde_yaml::to_string(configs)
        .map_err(|source| PaletteIoError::SerializeYaml { source })?;
    let path = config_dir.join("model-configs.yaml");
    fs::write(&path, data).map_err(|source| PaletteIoError::Write { path, source })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;
    use tempfile::TempDir;

    use super::{
        ExportOptions, ImportOptions, ImportStatus, MergeStrategy, PaletteFormat, export_palette,
        import_palette,
    };
    use crate::config::{AgentConfig, ModelConfigs, Palette, Reasoning};

    fn write_configs(config_dir: &TempDir, configs: &ModelConfigs) {
        let data = serde_yaml::to_string(configs).expect("serialize configs");
        fs::write(config_dir.path().join("model-configs.yaml"), data)
            .expect("write model-configs.yaml");
    }

    #[test]
    fn export_palette_serializes_json() {
        let config_dir = TempDir::new().expect("config dir");
        let mut palettes = std::collections::BTreeMap::new();
        palettes.insert(
            "alpha".to_string(),
            Palette {
                agents: std::collections::BTreeMap::from([(
                    "build".to_string(),
                    AgentConfig {
                        model: "openrouter/build".to_string(),
                        variant: Some("mini".to_string()),
                        reasoning: Some(Reasoning::Bool(true)),
                    },
                )]),
                mapping: std::collections::BTreeMap::from([("build-count".to_string(), json!(3))]),
            },
        );
        write_configs(&config_dir, &ModelConfigs { palettes });

        let output = export_palette(ExportOptions {
            name: "alpha".to_string(),
            format: PaletteFormat::Json,
            config_dir: config_dir.path().to_path_buf(),
        })
        .expect("export palette");

        let parsed: Palette = serde_json::from_str(&output.data).expect("parse json");
        assert_eq!(parsed.agents.len(), 1);
        assert_eq!(parsed.agents["build"].model, "openrouter/build");
        assert_eq!(parsed.mapping["build-count"], json!(3));
    }

    #[test]
    fn import_palette_aborts_on_conflict() {
        let config_dir = TempDir::new().expect("config dir");
        let mut palettes = std::collections::BTreeMap::new();
        palettes.insert(
            "alpha".to_string(),
            Palette {
                agents: std::collections::BTreeMap::from([(
                    "build".to_string(),
                    AgentConfig {
                        model: "openrouter/old".to_string(),
                        variant: None,
                        reasoning: None,
                    },
                )]),
                mapping: std::collections::BTreeMap::new(),
            },
        );
        write_configs(&config_dir, &ModelConfigs { palettes });

        let import_path = config_dir.path().join("import.yaml");
        fs::write(
            &import_path,
            r#"agents:
  build:
    model: openrouter/new
"#,
        )
        .expect("write import");

        let report = import_palette(ImportOptions {
            from: import_path,
            name: Some("alpha".to_string()),
            merge: MergeStrategy::Abort,
            dry_run: false,
            force: true,
            config_dir: config_dir.path().to_path_buf(),
        })
        .expect("import palette");

        assert_eq!(report.status, ImportStatus::Aborted);
        assert_eq!(report.conflicts, vec!["agent.build".to_string()]);
    }

    #[test]
    fn import_palette_merges_new_agents() {
        let config_dir = TempDir::new().expect("config dir");
        let mut palettes = std::collections::BTreeMap::new();
        palettes.insert(
            "alpha".to_string(),
            Palette {
                agents: std::collections::BTreeMap::from([(
                    "build".to_string(),
                    AgentConfig {
                        model: "openrouter/build".to_string(),
                        variant: None,
                        reasoning: None,
                    },
                )]),
                mapping: std::collections::BTreeMap::new(),
            },
        );
        write_configs(&config_dir, &ModelConfigs { palettes });

        let import_path = config_dir.path().join("import.yaml");
        fs::write(
            &import_path,
            r#"agents:
  review:
    model: openrouter/review
mapping:
  review-count: 2
"#,
        )
        .expect("write import");

        let report = import_palette(ImportOptions {
            from: import_path,
            name: Some("alpha".to_string()),
            merge: MergeStrategy::Merge,
            dry_run: false,
            force: true,
            config_dir: config_dir.path().to_path_buf(),
        })
        .expect("import palette");

        assert_eq!(report.status, ImportStatus::Applied);
        assert!(report.conflicts.is_empty());

        let configs = crate::config::load_model_configs(config_dir.path()).expect("load configs");
        let palette = configs.palettes.get("alpha").expect("alpha palette");
        assert!(palette.agents.contains_key("build"));
        assert!(palette.agents.contains_key("review"));
        assert_eq!(palette.mapping["review-count"], json!(2));
    }

    #[test]
    fn export_palette_serializes_yaml() {
        let config_dir = TempDir::new().expect("config dir");
        let mut palettes = std::collections::BTreeMap::new();
        palettes.insert(
            "beta".to_string(),
            Palette {
                agents: std::collections::BTreeMap::from([(
                    "review".to_string(),
                    AgentConfig {
                        model: "openrouter/review-v2".to_string(),
                        variant: Some("large".to_string()),
                        reasoning: Some(Reasoning::Bool(false)),
                    },
                )]),
                mapping: std::collections::BTreeMap::from([("timeout".to_string(), json!(30))]),
            },
        );
        write_configs(&config_dir, &ModelConfigs { palettes });

        let output = export_palette(ExportOptions {
            name: "beta".to_string(),
            format: PaletteFormat::Yaml,
            config_dir: config_dir.path().to_path_buf(),
        })
        .expect("export palette as yaml");

        let parsed: Palette = serde_yaml::from_str(&output.data).expect("parse yaml output");
        assert_eq!(parsed.agents.len(), 1);
        assert_eq!(parsed.agents["review"].model, "openrouter/review-v2");
        assert_eq!(parsed.agents["review"].variant.as_deref(), Some("large"));
        assert_eq!(parsed.mapping["timeout"], json!(30));
        assert!(output.lines > 0);
    }

    #[test]
    fn import_palette_overwrite_replaces_existing_agents() {
        let config_dir = TempDir::new().expect("config dir");
        let mut palettes = std::collections::BTreeMap::new();
        palettes.insert(
            "gamma".to_string(),
            Palette {
                agents: std::collections::BTreeMap::from([
                    (
                        "build".to_string(),
                        AgentConfig {
                            model: "openrouter/old-build".to_string(),
                            variant: None,
                            reasoning: None,
                        },
                    ),
                    (
                        "lint".to_string(),
                        AgentConfig {
                            model: "openrouter/old-lint".to_string(),
                            variant: None,
                            reasoning: None,
                        },
                    ),
                ]),
                mapping: std::collections::BTreeMap::from([("retries".to_string(), json!(1))]),
            },
        );
        write_configs(&config_dir, &ModelConfigs { palettes });

        let import_path = config_dir.path().join("overwrite.yaml");
        fs::write(
            &import_path,
            r#"agents:
  build:
    model: openrouter/new-build
    variant: turbo
mapping:
  retries: 5
  extra-flag: true
"#,
        )
        .expect("write import file");

        let report = import_palette(ImportOptions {
            from: import_path,
            name: Some("gamma".to_string()),
            merge: MergeStrategy::Overwrite,
            dry_run: false,
            force: true,
            config_dir: config_dir.path().to_path_buf(),
        })
        .expect("import palette with overwrite");

        assert_eq!(report.status, ImportStatus::Applied);
        assert!(!report.created);

        let configs = crate::config::load_model_configs(config_dir.path()).expect("load configs");
        let palette = configs.palettes.get("gamma").expect("gamma palette");

        // Overwrite replaces entirely: only incoming agents/mapping survive
        assert_eq!(palette.agents.len(), 1);
        assert_eq!(palette.agents["build"].model, "openrouter/new-build");
        assert_eq!(palette.agents["build"].variant.as_deref(), Some("turbo"));
        assert!(!palette.agents.contains_key("lint"));
        assert_eq!(palette.mapping["retries"], json!(5));
        assert_eq!(palette.mapping["extra-flag"], json!(true));
        assert_eq!(palette.mapping.len(), 2);
    }

    #[test]
    fn import_malformed_yaml_returns_parse_error() {
        let config_dir = TempDir::new().expect("config dir");
        write_configs(
            &config_dir,
            &ModelConfigs {
                palettes: std::collections::BTreeMap::new(),
            },
        );

        let import_path = config_dir.path().join("broken.yaml");
        fs::write(
            &import_path,
            // Structurally invalid YAML: tab indentation mixed with invalid mapping
            "agents:\n  build:\n    model: [unterminated\n    :\n",
        )
        .expect("write malformed import file");

        let result = import_palette(ImportOptions {
            from: import_path,
            name: Some("bad".to_string()),
            merge: MergeStrategy::Merge,
            dry_run: false,
            force: true,
            config_dir: config_dir.path().to_path_buf(),
        });

        let err = match result {
            Err(e) => e,
            Ok(_) => panic!("expected parse error for malformed YAML"),
        };
        let msg = err.to_string();
        assert!(
            msg.contains("parse yaml palette") || msg.contains("parse palette"),
            "expected parse error, got: {msg}"
        );
    }
}
