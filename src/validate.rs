//! Template and palette validation engine.
//!
//! This module checks template files and model-config palettes for
//! structural errors, unknown placeholders, ambiguous aliases, and
//! missing model fields.  Results are collected into a [`Report`]
//! containing [`Finding`]s at either [`Severity::Error`] or
//! [`Severity::Warning`] level.
//!
//! Internally a `ValidatorBuilder` typestate drives the pipeline:
//!
//! ```text
//!  Start
//!    │
//!    ▼
//!  PalettesLoaded      load & parse model-configs.yaml
//!    │
//!    ▼
//!  TemplatesDiscovered  resolve glob / explicit template paths
//!    │
//!    ▼
//!  PlaceholdersScanned  walk JSON values, collect {{…}} uses
//!    │
//!    ▼
//!  ReportReady          cross-check placeholders against palette
//!    │
//!    ▼
//!  Report               final counts + findings list
//! ```

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use glob::glob;
use regex::Regex;
use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

use crate::config::{AgentConfig, ModelConfigs, Palette, Reasoning};
use crate::template::{load_template, TemplateError};

#[derive(Debug, Clone)]
pub struct ValidateOpts {
    pub templates: Vec<String>,
    pub palettes_path: Option<PathBuf>,
    pub strict: bool,
    pub env_allow: Option<bool>,
    pub env_mask_logs: Option<bool>,
    pub schema: bool,
}

#[derive(Debug, Serialize)]
pub struct Counts {
    pub errors: u32,
    pub warnings: u32,
}

#[derive(Debug, Clone)]
pub struct Finding {
    pub file: String,
    pub path: String,
    pub kind: String,
    pub message: String,
    pub severity: Severity,
}

#[derive(Debug)]
pub struct Report {
    pub counts: Counts,
    pub findings: Vec<Finding>,
}

#[derive(Debug, Error)]
pub enum ValidateError {
    #[error("template error: {0}")]
    Template(#[from] TemplateError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("yaml error: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("regex error: {0}")]
    Regex(#[from] regex::Error),
    #[error("other: {0}")]
    Other(String),
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Severity {
    Error,
    Warning,
}

impl Severity {
    fn as_label(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
        }
    }
}

#[derive(Default)]
struct ReportBuilder {
    findings: Vec<Finding>,
    errors: u32,
    warnings: u32,
}

impl ReportBuilder {
    fn push(
        &mut self,
        severity: Severity,
        file: String,
        path: String,
        kind: &str,
        message: String,
    ) {
        match severity {
            Severity::Error => self.errors += 1,
            Severity::Warning => self.warnings += 1,
        }
        self.findings.push(Finding {
            file,
            path,
            kind: kind.to_string(),
            message,
            severity,
        });
    }

    fn warn_or_error(
        &mut self,
        strict: bool,
        file: String,
        path: String,
        kind: &str,
        message: String,
    ) {
        let severity = if strict {
            Severity::Error
        } else {
            Severity::Warning
        };
        self.push(severity, file, path, kind, message);
    }

    fn build(self) -> Report {
        Report {
            counts: Counts {
                errors: self.errors,
                warnings: self.warnings,
            },
            findings: self.findings,
        }
    }
}

#[derive(Debug, Clone)]
struct PlaceholderUse {
    key: String,
    path: String,
    is_full: bool,
    json_key: Option<String>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum Availability {
    Found,
    Missing,
    MissingVariant,
    MissingReasoning,
    MissingReasoningDetail,
}

struct Start;

struct PalettesLoaded {
    palette_info: Option<PaletteInfo>,
}

struct TemplatesDiscovered {
    palette_info: Option<PaletteInfo>,
    template_paths: Vec<PathBuf>,
}

struct PlaceholdersScanned {
    palette_info: Option<PaletteInfo>,
    scans: Vec<TemplateScan>,
}

struct ReportReady;

struct ValidatorBuilder<State> {
    config_dir: PathBuf,
    opts: ValidateOpts,
    report: ReportBuilder,
    placeholder_regex: Regex,
    palettes_path: PathBuf,
    state: State,
}

struct TemplateScan {
    file_display: String,
    uses: Vec<PlaceholderUse>,
}

impl ValidatorBuilder<Start> {
    fn new(config_dir: &Path, opts: ValidateOpts) -> Result<Self, ValidateError> {
        let mut report = ReportBuilder::default();
        let placeholder_regex = Regex::new(r"\{\{\s*([^\}]+?)\s*\}\}")?;
        if opts.schema {
            report.warn_or_error(
                opts.strict,
                "validate".to_string(),
                "$.schema".to_string(),
                "schema-not-implemented",
                "schema validation is not implemented".to_string(),
            );
        }
        if opts.env_allow == Some(true) || opts.env_mask_logs == Some(true) {
            let mut details = Vec::new();
            if let Some(value) = opts.env_allow.filter(|v| *v) {
                details.push(format!("env_allow={value}"));
            }
            if let Some(value) = opts.env_mask_logs.filter(|v| *v) {
                details.push(format!("env_mask_logs={value}"));
            }
            let message = if details.is_empty() {
                "env flags are not implemented for validation".to_string()
            } else {
                format!(
                    "env flags are not implemented for validation ({})",
                    details.join(", ")
                )
            };
            report.warn_or_error(
                opts.strict,
                "validate".to_string(),
                "$.env".to_string(),
                "env-flags-not-implemented",
                message,
            );
        }

        let palettes_path = opts
            .palettes_path
            .clone()
            .unwrap_or_else(|| config_dir.join("model-configs.yaml"));

        Ok(Self {
            config_dir: config_dir.to_path_buf(),
            opts,
            report,
            placeholder_regex,
            palettes_path,
            state: Start,
        })
    }

    fn load_palettes(mut self) -> ValidatorBuilder<PalettesLoaded> {
        let palettes_result = load_palettes(&self.palettes_path);
        let palette_info = match palettes_result {
            Ok(configs) => Some(build_palette_info(
                &configs,
                &self.palettes_path,
                &mut self.report,
                self.opts.strict,
                &self.config_dir,
            )),
            Err(err) => {
                self.report.push(
                    Severity::Error,
                    display_path(&self.palettes_path, &self.config_dir),
                    "$".to_string(),
                    "invalid-palettes",
                    err.to_string(),
                );
                None
            }
        };

        ValidatorBuilder {
            config_dir: self.config_dir,
            opts: self.opts,
            report: self.report,
            placeholder_regex: self.placeholder_regex,
            palettes_path: self.palettes_path,
            state: PalettesLoaded { palette_info },
        }
    }
}

impl ValidatorBuilder<PalettesLoaded> {
    fn discover_templates(
        mut self,
    ) -> Result<ValidatorBuilder<TemplatesDiscovered>, ValidateError> {
        let template_paths = resolve_template_paths(
            &self.config_dir,
            &self.opts.templates,
            &mut self.report,
            self.opts.strict,
        )?;
        if template_paths.is_empty() {
            self.report.warn_or_error(
                self.opts.strict,
                display_path(&self.config_dir.join("template.d"), &self.config_dir),
                "$".to_string(),
                "missing-templates",
                "no templates found to validate".to_string(),
            );
        }
        Ok(ValidatorBuilder {
            config_dir: self.config_dir,
            opts: self.opts,
            report: self.report,
            placeholder_regex: self.placeholder_regex,
            palettes_path: self.palettes_path,
            state: TemplatesDiscovered {
                palette_info: self.state.palette_info,
                template_paths,
            },
        })
    }
}

impl ValidatorBuilder<TemplatesDiscovered> {
    fn scan_templates(mut self) -> Result<ValidatorBuilder<PlaceholdersScanned>, ValidateError> {
        let mut scans = Vec::new();
        for template_path in &self.state.template_paths {
            let file_display = display_path(template_path, &self.config_dir);
            match load_template(template_path) {
                Ok(value) => {
                    let mut uses = Vec::new();
                    scan_placeholders(
                        &value,
                        "$".to_string(),
                        None,
                        &mut uses,
                        &file_display,
                        &self.placeholder_regex,
                        &mut self.report,
                        self.opts.strict,
                    );
                    scans.push(TemplateScan { file_display, uses });
                }
                Err(err) => {
                    self.report.push(
                        Severity::Error,
                        file_display,
                        "$".to_string(),
                        "invalid-template",
                        err.to_string(),
                    );
                }
            }
        }

        Ok(ValidatorBuilder {
            config_dir: self.config_dir,
            opts: self.opts,
            report: self.report,
            placeholder_regex: self.placeholder_regex,
            palettes_path: self.palettes_path,
            state: PlaceholdersScanned {
                palette_info: self.state.palette_info,
                scans,
            },
        })
    }
}

impl ValidatorBuilder<PlaceholdersScanned> {
    fn validate_placeholders(mut self) -> ValidatorBuilder<ReportReady> {
        if let Some(info) = self.state.palette_info.as_ref() {
            for scan in &self.state.scans {
                validate_placeholders(
                    &scan.uses,
                    &info.palettes,
                    &scan.file_display,
                    &mut self.report,
                    self.opts.strict,
                );
            }
        }

        ValidatorBuilder {
            config_dir: self.config_dir,
            opts: self.opts,
            report: self.report,
            placeholder_regex: self.placeholder_regex,
            palettes_path: self.palettes_path,
            state: ReportReady,
        }
    }
}

impl ValidatorBuilder<ReportReady> {
    fn build(self) -> Result<Report, ValidateError> {
        Ok(self.report.build())
    }
}

/// Validate the config directory for template and palette issues.
#[allow(dead_code)]
fn validate_dir_legacy(config_dir: &Path, opts: ValidateOpts) -> Result<Report, ValidateError> {
    let mut report = ReportBuilder::default();
    let placeholder_regex = Regex::new(r"\{\{\s*([^\}]+?)\s*\}\}")?;

    if opts.schema {
        report.warn_or_error(
            opts.strict,
            "validate".to_string(),
            "$.schema".to_string(),
            "schema-not-implemented",
            "schema validation is not implemented".to_string(),
        );
    }
    if opts.env_allow == Some(true) || opts.env_mask_logs == Some(true) {
        let mut details = Vec::new();
        if let Some(value) = opts.env_allow.filter(|v| *v) {
            details.push(format!("env_allow={value}"));
        }
        if let Some(value) = opts.env_mask_logs.filter(|v| *v) {
            details.push(format!("env_mask_logs={value}"));
        }
        let message = if details.is_empty() {
            "env flags are not implemented for validation".to_string()
        } else {
            format!(
                "env flags are not implemented for validation ({})",
                details.join(", ")
            )
        };
        report.warn_or_error(
            opts.strict,
            "validate".to_string(),
            "$.env".to_string(),
            "env-flags-not-implemented",
            message,
        );
    }

    let palettes_path = opts
        .palettes_path
        .clone()
        .unwrap_or_else(|| config_dir.join("model-configs.yaml"));
    let palettes_result = load_palettes(&palettes_path);

    let palette_info = match palettes_result {
        Ok(configs) => Some(build_palette_info(
            &configs,
            &palettes_path,
            &mut report,
            opts.strict,
            config_dir,
        )),
        Err(err) => {
            report.push(
                Severity::Error,
                display_path(&palettes_path, config_dir),
                "$".to_string(),
                "invalid-palettes",
                err.to_string(),
            );
            None
        }
    };

    let template_paths =
        resolve_template_paths(config_dir, &opts.templates, &mut report, opts.strict)?;
    if template_paths.is_empty() {
        report.warn_or_error(
            opts.strict,
            display_path(&config_dir.join("template.d"), config_dir),
            "$".to_string(),
            "missing-templates",
            "no templates found to validate".to_string(),
        );
    }

    for template_path in template_paths {
        let file_display = display_path(&template_path, config_dir);
        match load_template(&template_path) {
            Ok(value) => {
                let mut uses = Vec::new();
                scan_placeholders(
                    &value,
                    "$".to_string(),
                    None,
                    &mut uses,
                    &file_display,
                    &placeholder_regex,
                    &mut report,
                    opts.strict,
                );

                if let Some(info) = palette_info.as_ref() {
                    validate_placeholders(
                        &uses,
                        &info.palettes,
                        &file_display,
                        &mut report,
                        opts.strict,
                    );
                }
            }
            Err(err) => {
                report.push(
                    Severity::Error,
                    file_display,
                    "$".to_string(),
                    "invalid-template",
                    err.to_string(),
                );
            }
        }
    }

    Ok(report.build())
}

/// Format a report in human-friendly text.
pub fn format_report_text(report: &Report) -> String {
    if report.counts.errors == 0 && report.counts.warnings == 0 {
        return "Validation succeeded: no issues found".to_string();
    }
    let headline = if report.counts.errors > 0 {
        format!(
            "Validation failed: {} errors, {} warnings\n",
            report.counts.errors, report.counts.warnings
        )
    } else {
        format!(
            "Validation succeeded with {} warnings\n",
            report.counts.warnings
        )
    };

    let mut out = headline;
    for f in &report.findings {
        out.push_str(&format!(
            "{} {} @ {}: {}\n",
            f.severity.as_label().to_uppercase(),
            f.file,
            f.path,
            f.message
        ));
    }
    out
}

/// Validate the config directory for template and palette issues.
pub fn validate_dir(config_dir: &Path, opts: ValidateOpts) -> Result<Report, ValidateError> {
    ValidatorBuilder::new(config_dir, opts)?
        .load_palettes()
        .discover_templates()?
        .scan_templates()?
        .validate_placeholders()
        .build()
}

/// Format a report suitable for JSON serialization by the caller.
pub fn format_report_json(report: &Report) -> JsonReport {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    for finding in &report.findings {
        let entry = JsonFinding {
            file: finding.file.clone(),
            path: finding.path.clone(),
            kind: finding.kind.clone(),
            message: finding.message.clone(),
        };
        match finding.severity {
            Severity::Error => errors.push(entry),
            Severity::Warning => warnings.push(entry),
        }
    }

    JsonReport { errors, warnings }
}

#[derive(Debug, Serialize)]
pub struct JsonFinding {
    pub file: String,
    pub path: String,
    pub kind: String,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct JsonReport {
    pub errors: Vec<JsonFinding>,
    pub warnings: Vec<JsonFinding>,
}

struct PaletteInfo {
    palettes: Vec<PaletteSummary>,
}

struct PaletteSummary {
    name: String,
    palette: Palette,
    mapping_keys: HashSet<String>,
}

fn load_palettes(path: &Path) -> Result<ModelConfigs, ValidateError> {
    let data = fs::read_to_string(path)?;
    let configs: ModelConfigs = serde_yaml::from_str(&data)?;
    Ok(configs)
}

fn build_palette_info(
    configs: &ModelConfigs,
    palettes_path: &Path,
    report: &mut ReportBuilder,
    strict: bool,
    config_dir: &Path,
) -> PaletteInfo {
    let mut palettes = Vec::new();
    for (name, palette) in &configs.palettes {
        let mapping_keys: HashSet<String> = palette.mapping.keys().cloned().collect();
        detect_ambiguous_aliases(name, palette, palettes_path, report, strict, config_dir);
        palettes.push(PaletteSummary {
            name: name.clone(),
            palette: palette.clone(),
            mapping_keys,
        });
    }
    PaletteInfo { palettes }
}

fn detect_ambiguous_aliases(
    palette_name: &str,
    palette: &Palette,
    palettes_path: &Path,
    report: &mut ReportBuilder,
    strict: bool,
    config_dir: &Path,
) {
    let mut generated = HashMap::<String, String>::new();
    for agent in palette.agents.keys() {
        generated.insert(agent.clone(), agent.clone());
        generated.insert(format!("agent-{agent}-model"), agent.clone());
        generated.insert(format!("agent-{agent}-variant"), agent.clone());
        generated.insert(format!("{agent}-variant"), agent.clone());
        generated.insert(format!("agent-{agent}-reasoning-effort"), agent.clone());
        generated.insert(format!("agent-{agent}-text-verbosity"), agent.clone());
    }

    for key in palette.mapping.keys() {
        if let Some(agent) = generated.get(key) {
            let path = format!("$.palettes.{palette_name}.mapping.{key}");
            let message =
                format!("mapping key '{key}' conflicts with generated alias for agent '{agent}'");
            report.warn_or_error(
                strict,
                display_path_with_fallback(palettes_path, config_dir),
                path,
                "ambiguous-alias",
                message,
            );
        }
    }
}

fn resolve_template_paths(
    config_dir: &Path,
    patterns: &[String],
    report: &mut ReportBuilder,
    strict: bool,
) -> Result<Vec<PathBuf>, ValidateError> {
    let mut paths = BTreeSet::new();
    if patterns.is_empty() {
        let template_dir = config_dir.join("template.d");
        let entries = match fs::read_dir(&template_dir) {
            Ok(entries) => entries,
            Err(err) => {
                report.push(
                    Severity::Error,
                    display_path(&template_dir, config_dir),
                    "$".to_string(),
                    "missing-template-dir",
                    format!("{err}"),
                );
                return Ok(Vec::new());
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(err) => {
                    report.push(
                        Severity::Error,
                        display_path(&template_dir, config_dir),
                        "$".to_string(),
                        "template-dir-read",
                        format!("{err}"),
                    );
                    continue;
                }
            };
            let path = entry.path();
            if is_template_path(&path) {
                paths.insert(path);
            } else if path.is_file() {
                report.warn_or_error(
                    strict,
                    display_path(&path, config_dir),
                    "$".to_string(),
                    "unsupported-template",
                    "template extension is not supported".to_string(),
                );
            }
        }
        return Ok(paths.into_iter().collect());
    }

    for pattern in patterns {
        let mut matched = false;
        let resolved = if Path::new(pattern).is_absolute() {
            pattern.clone()
        } else {
            config_dir.join(pattern).to_string_lossy().to_string()
        };
        match glob(&resolved) {
            Ok(entries) => {
                for entry in entries {
                    match entry {
                        Ok(path) => {
                            matched = true;
                            if path.is_file() {
                                if is_template_path(&path) {
                                    paths.insert(path);
                                } else {
                                    report.warn_or_error(
                                        strict,
                                        display_path(&path, config_dir),
                                        "$".to_string(),
                                        "unsupported-template",
                                        "template extension is not supported".to_string(),
                                    );
                                }
                            }
                        }
                        Err(err) => {
                            report.push(
                                Severity::Error,
                                resolved.clone(),
                                "$".to_string(),
                                "template-glob",
                                err.to_string(),
                            );
                        }
                    }
                }
            }
            Err(err) => {
                report.push(
                    Severity::Error,
                    resolved.clone(),
                    "$".to_string(),
                    "template-glob",
                    err.to_string(),
                );
            }
        }
        if !matched {
            report.warn_or_error(
                strict,
                resolved,
                "$".to_string(),
                "missing-template",
                "no templates matched glob".to_string(),
            );
        }
    }

    Ok(paths.into_iter().collect())
}

fn is_template_path(path: &Path) -> bool {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) => matches!(ext.to_ascii_lowercase().as_str(), "json" | "yaml" | "yml"),
        None => false,
    }
}

#[allow(clippy::too_many_arguments, clippy::only_used_in_recursion)]
fn scan_placeholders(
    value: &Value,
    path: String,
    json_key: Option<String>,
    uses: &mut Vec<PlaceholderUse>,
    file: &str,
    regex: &Regex,
    report: &mut ReportBuilder,
    strict: bool,
) {
    match value {
        Value::Object(map) => {
            let base_path = path.trim_end_matches('.');
            for (key, child) in map {
                let child_path = format!("{base_path}.{key}");
                scan_placeholders(
                    child,
                    child_path,
                    Some(key.clone()),
                    uses,
                    file,
                    regex,
                    report,
                    strict,
                );
            }
        }
        Value::Array(items) => {
            let base_path = path.trim_end_matches('.');
            for (index, child) in items.iter().enumerate() {
                let child_path = format!("{base_path}[{index}]");
                scan_placeholders(child, child_path, None, uses, file, regex, report, strict);
            }
        }
        Value::String(text) => {
            let mut matches = regex.captures_iter(text).peekable();
            let has_braces = text.contains("{{") || text.contains("}}");
            if matches.peek().is_none() && has_braces {
                report.push(
                    Severity::Error,
                    file.to_string(),
                    path.clone(),
                    "malformed-placeholder",
                    "placeholder braces found but no valid placeholder".to_string(),
                );
                return;
            }

            for capture in regex.captures_iter(text) {
                let raw = capture.get(0).map(|m| m.as_str()).unwrap_or("");
                let key = capture.get(1).map(|m| m.as_str().trim()).unwrap_or("");
                if key.is_empty() {
                    report.push(
                        Severity::Error,
                        file.to_string(),
                        path.clone(),
                        "malformed-placeholder",
                        "placeholder key is empty".to_string(),
                    );
                    continue;
                }
                let trimmed = text.trim();
                let is_full = trimmed == raw;
                uses.push(PlaceholderUse {
                    key: key.to_string(),
                    path: path.clone(),
                    is_full,
                    json_key: json_key.clone(),
                });
            }
        }
        _ => {}
    }
}

fn validate_placeholders(
    uses: &[PlaceholderUse],
    palettes: &[PaletteSummary],
    file: &str,
    report: &mut ReportBuilder,
    strict: bool,
) {
    for usage in uses {
        let mut found = false;
        let mut missing_palettes = Vec::new();
        let mut missing_variant = Vec::new();
        let mut missing_reasoning = Vec::new();
        let mut missing_reasoning_detail = Vec::new();

        for palette in palettes {
            match placeholder_availability(&usage.key, &palette.palette, &palette.mapping_keys) {
                Availability::Found => {
                    found = true;
                }
                Availability::Missing => {
                    missing_palettes.push(palette.name.clone());
                }
                Availability::MissingVariant => {
                    missing_variant.push(palette.name.clone());
                }
                Availability::MissingReasoning => {
                    missing_reasoning.push(palette.name.clone());
                }
                Availability::MissingReasoningDetail => {
                    missing_reasoning_detail.push(palette.name.clone());
                }
            }
        }

        let is_optional_variant = usage.is_full
            && usage
                .json_key
                .as_deref()
                .is_some_and(|key| key == "variant")
            && usage.key.ends_with("-variant");

        if !found {
            if is_optional_variant {
                report.warn_or_error(
                    strict,
                    file.to_string(),
                    usage.path.clone(),
                    "missing-variant",
                    format!("variant placeholder '{}' is missing", usage.key),
                );
            } else {
                report.push(
                    Severity::Error,
                    file.to_string(),
                    usage.path.clone(),
                    "unknown-placeholder",
                    format!("unknown placeholder '{}'", usage.key),
                );
            }
            continue;
        }

        if !missing_palettes.is_empty() {
            report.warn_or_error(
                strict,
                file.to_string(),
                usage.path.clone(),
                "palette-mismatch",
                format!(
                    "placeholder '{}' missing from palettes: {}",
                    usage.key,
                    missing_palettes.join(", ")
                ),
            );
        }
        if !missing_variant.is_empty() {
            let kind = if is_optional_variant {
                "missing-variant"
            } else {
                "palette-mismatch"
            };
            report.warn_or_error(
                strict,
                file.to_string(),
                usage.path.clone(),
                kind,
                format!(
                    "variant placeholder '{}' missing from palettes: {}",
                    usage.key,
                    missing_variant.join(", ")
                ),
            );
        }
        if !missing_reasoning.is_empty() {
            report.warn_or_error(
                strict,
                file.to_string(),
                usage.path.clone(),
                "missing-reasoning",
                format!(
                    "reasoning placeholder '{}' missing from palettes: {}",
                    usage.key,
                    missing_reasoning.join(", ")
                ),
            );
        }
        if !missing_reasoning_detail.is_empty() {
            report.warn_or_error(
                strict,
                file.to_string(),
                usage.path.clone(),
                "missing-reasoning",
                format!(
                    "reasoning placeholder '{}' missing from palettes: {}",
                    usage.key,
                    missing_reasoning_detail.join(", ")
                ),
            );
        }
    }
}

fn placeholder_availability(
    key: &str,
    palette: &Palette,
    mapping_keys: &HashSet<String>,
) -> Availability {
    if mapping_keys.contains(key) {
        return Availability::Found;
    }

    if let Some(agent) = key.strip_prefix("agent-") {
        if let Some(name) = agent.strip_suffix("-model") {
            return availability_for_agent(name, palette, AgentKey::Model);
        }
        if let Some(name) = agent.strip_suffix("-variant") {
            return availability_for_agent(name, palette, AgentKey::Variant);
        }
        if let Some(name) = agent.strip_suffix("-reasoning-effort") {
            return availability_for_agent(name, palette, AgentKey::ReasoningEffort);
        }
        if let Some(name) = agent.strip_suffix("-text-verbosity") {
            return availability_for_agent(name, palette, AgentKey::TextVerbosity);
        }
    }

    if let Some(name) = key.strip_suffix("-variant") {
        return availability_for_agent(name, palette, AgentKey::Variant);
    }

    if palette.agents.contains_key(key) {
        return Availability::Found;
    }

    Availability::Missing
}

enum AgentKey {
    Model,
    Variant,
    ReasoningEffort,
    TextVerbosity,
}

fn availability_for_agent(name: &str, palette: &Palette, key: AgentKey) -> Availability {
    let Some(agent) = palette.agents.get(name) else {
        return Availability::Missing;
    };
    match key {
        AgentKey::Model => Availability::Found,
        AgentKey::Variant => agent
            .variant
            .as_ref()
            .map(|_| Availability::Found)
            .unwrap_or(Availability::MissingVariant),
        AgentKey::ReasoningEffort => reasoning_effort_status(agent),
        AgentKey::TextVerbosity => reasoning_text_status(agent),
    }
}

fn reasoning_effort_status(agent: &AgentConfig) -> Availability {
    match &agent.reasoning {
        Some(Reasoning::Bool(true)) => Availability::Found,
        Some(Reasoning::Bool(false)) | None => Availability::MissingReasoning,
        Some(Reasoning::Object(cfg)) => cfg
            .effort
            .as_ref()
            .map(|_| Availability::Found)
            .unwrap_or(Availability::MissingReasoningDetail),
    }
}

fn reasoning_text_status(agent: &AgentConfig) -> Availability {
    match &agent.reasoning {
        Some(Reasoning::Object(cfg)) => cfg
            .text_verbosity
            .as_ref()
            .map(|_| Availability::Found)
            .unwrap_or(Availability::MissingReasoningDetail),
        Some(Reasoning::Bool(_)) | None => Availability::MissingReasoning,
    }
}

fn display_path(path: &Path, base: &Path) -> String {
    path.strip_prefix(base)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string()
}

fn display_path_with_fallback(path: &Path, base: &Path) -> String {
    if let Ok(relative) = path.strip_prefix(base) {
        return relative.to_string_lossy().to_string();
    }
    path.to_string_lossy().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_palettes(path: &Path) {
        let yaml = r#"
palettes:
  default:
    agents:
      build:
        model: openrouter/openai/gpt-4o
        variant: mini
"#;
        fs::write(path.join("model-configs.yaml"), yaml).expect("write palettes");
    }

    #[test]
    fn validate_detects_malformed_json() {
        let temp_dir = TempDir::new().expect("temp dir");
        let config_dir = temp_dir.path();
        write_palettes(config_dir);
        let template_dir = config_dir.join("template.d");
        fs::create_dir_all(&template_dir).expect("template dir");
        fs::write(template_dir.join("bad.json"), "{ invalid ").expect("write json");

        let opts = ValidateOpts {
            templates: Vec::new(),
            palettes_path: None,
            strict: false,
            env_allow: None,
            env_mask_logs: None,
            schema: false,
        };

        let report = validate_dir(config_dir, opts).expect("validate");
        assert!(report.counts.errors > 0);
        assert!(report.findings.iter().any(|f| f.kind == "invalid-template"));
    }

    #[test]
    fn validate_detects_unknown_placeholder() {
        let temp_dir = TempDir::new().expect("temp dir");
        let config_dir = temp_dir.path();
        write_palettes(config_dir);
        let template_dir = config_dir.join("template.d");
        fs::create_dir_all(&template_dir).expect("template dir");
        fs::write(
            template_dir.join("default.json"),
            r#"{ "agent": { "build": { "model": "{{missing}}" } } }"#,
        )
        .expect("write template");

        let opts = ValidateOpts {
            templates: Vec::new(),
            palettes_path: None,
            strict: false,
            env_allow: None,
            env_mask_logs: None,
            schema: false,
        };

        let report = validate_dir(config_dir, opts).expect("validate");
        assert!(report.counts.errors > 0);
        assert!(report
            .findings
            .iter()
            .any(|f| f.kind == "unknown-placeholder"));
    }

    #[test]
    fn validate_warns_on_missing_variant() {
        let temp_dir = TempDir::new().expect("temp dir");
        let config_dir = temp_dir.path();
        let yaml = r#"
palettes:
  default:
    agents:
      build:
        model: openrouter/openai/gpt-4o
"#;
        fs::write(config_dir.join("model-configs.yaml"), yaml).expect("write palettes");
        let template_dir = config_dir.join("template.d");
        fs::create_dir_all(&template_dir).expect("template dir");
        fs::write(
            template_dir.join("default.json"),
            r#"{ "agent": { "build": { "variant": "{{build-variant}}" } } }"#,
        )
        .expect("write template");

        let opts = ValidateOpts {
            templates: Vec::new(),
            palettes_path: None,
            strict: false,
            env_allow: None,
            env_mask_logs: None,
            schema: false,
        };
        let report = validate_dir(config_dir, opts).expect("validate");
        assert_eq!(report.counts.errors, 0);
        assert!(report.counts.warnings > 0);
    }

    fn write_template(config_dir: &Path, name: &str, contents: &str) {
        let template_dir = config_dir.join("template.d");
        fs::create_dir_all(&template_dir).expect("template dir");
        fs::write(template_dir.join(name), contents).expect("write template");
    }

    #[test]
    fn validate_warns_on_schema_flag() {
        let temp_dir = TempDir::new().expect("temp dir");
        let config_dir = temp_dir.path();
        write_palettes(config_dir);
        write_template(
            config_dir,
            "ok.json",
            r#"{ "agent": { "build": { "model": "{{build}}" } } }"#,
        );

        let opts = ValidateOpts {
            templates: Vec::new(),
            palettes_path: None,
            strict: false,
            env_allow: None,
            env_mask_logs: None,
            schema: true,
        };

        let report = validate_dir(config_dir, opts).expect("validate");
        assert_eq!(report.counts.errors, 0);
        assert!(report
            .findings
            .iter()
            .any(|f| f.kind == "schema-not-implemented"));
    }

    #[test]
    fn validate_warns_on_env_flags() {
        let temp_dir = TempDir::new().expect("temp dir");
        let config_dir = temp_dir.path();
        write_palettes(config_dir);
        write_template(
            config_dir,
            "ok.json",
            r#"{ "agent": { "build": { "model": "{{build}}" } } }"#,
        );

        let opts = ValidateOpts {
            templates: Vec::new(),
            palettes_path: None,
            strict: false,
            env_allow: Some(true),
            env_mask_logs: None,
            schema: false,
        };

        let report = validate_dir(config_dir, opts).expect("validate");
        assert_eq!(report.counts.errors, 0);
        assert!(report
            .findings
            .iter()
            .any(|f| f.kind == "env-flags-not-implemented"));
    }

    #[test]
    fn validate_does_not_warn_on_env_flags_false() {
        let temp_dir = TempDir::new().expect("temp dir");
        let config_dir = temp_dir.path();
        write_palettes(config_dir);
        write_template(
            config_dir,
            "ok.json",
            r#"{ "agent": { "build": { "model": "{{build}}" } } }"#,
        );

        let opts = ValidateOpts {
            templates: Vec::new(),
            palettes_path: None,
            strict: false,
            env_allow: Some(false),
            env_mask_logs: None,
            schema: false,
        };

        let report = validate_dir(config_dir, opts).expect("validate");
        assert!(report
            .findings
            .iter()
            .all(|f| f.kind != "env-flags-not-implemented"));
    }

    #[test]
    fn validate_clean_config_produces_no_errors() {
        let temp_dir = TempDir::new().expect("temp dir");
        let config_dir = temp_dir.path();
        write_palettes(config_dir);
        write_template(
            config_dir,
            "clean.json",
            r#"{ "agent": { "build": { "model": "{{agent-build-model}}" } } }"#,
        );

        let opts = ValidateOpts {
            templates: Vec::new(),
            palettes_path: None,
            strict: false,
            env_allow: None,
            env_mask_logs: None,
            schema: false,
        };

        let report = validate_dir(config_dir, opts).expect("validate");
        assert_eq!(
            report.counts.errors,
            0,
            "expected zero errors but got: {:?}",
            report
                .findings
                .iter()
                .filter(|f| f.severity == Severity::Error)
                .map(|f| format!("{}: {}", f.kind, f.message))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            report.counts.warnings,
            0,
            "expected zero warnings but got: {:?}",
            report
                .findings
                .iter()
                .filter(|f| f.severity == Severity::Warning)
                .map(|f| format!("{}: {}", f.kind, f.message))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn validate_json_report_structure() {
        let temp_dir = TempDir::new().expect("temp dir");
        let config_dir = temp_dir.path();
        write_palettes(config_dir);
        write_template(
            config_dir,
            "default.json",
            r#"{ "agent": { "build": { "model": "{{unknown-agent}}" } } }"#,
        );

        let opts = ValidateOpts {
            templates: Vec::new(),
            palettes_path: None,
            strict: false,
            env_allow: None,
            env_mask_logs: None,
            schema: false,
        };

        let report = validate_dir(config_dir, opts).expect("validate");
        let json_report = format_report_json(&report);
        let serialized = serde_json::to_value(&json_report).expect("serialize json report");

        // Top-level must have "errors" and "warnings" arrays
        assert!(serialized.get("errors").is_some(), "missing 'errors' key");
        assert!(
            serialized.get("warnings").is_some(),
            "missing 'warnings' key"
        );
        assert!(serialized["errors"].is_array(), "'errors' is not an array");
        assert!(
            serialized["warnings"].is_array(),
            "'warnings' is not an array"
        );

        // The unknown placeholder should appear in errors
        let errors = serialized["errors"].as_array().unwrap();
        assert!(!errors.is_empty(), "expected at least one error finding");

        let first = &errors[0];
        assert!(first.get("file").is_some(), "finding missing 'file'");
        assert!(first.get("path").is_some(), "finding missing 'path'");
        assert!(first.get("kind").is_some(), "finding missing 'kind'");
        assert!(first.get("message").is_some(), "finding missing 'message'");
        assert_eq!(first["kind"].as_str().unwrap(), "unknown-placeholder");
    }

    #[test]
    fn validate_missing_model_in_palette_produces_finding() {
        let temp_dir = TempDir::new().expect("temp dir");
        let config_dir = temp_dir.path();

        // Two palettes: "default" has agent "build", "alt" does not.
        let yaml = r#"
palettes:
  default:
    agents:
      build:
        model: openrouter/openai/gpt-4o
  alt:
    agents:
      deploy:
        model: openrouter/anthropic/claude-3.5-sonnet
"#;
        fs::write(config_dir.join("model-configs.yaml"), yaml).expect("write palettes");
        write_template(
            config_dir,
            "default.json",
            r#"{ "agent": { "build": { "model": "{{agent-build-model}}" } } }"#,
        );

        let opts = ValidateOpts {
            templates: Vec::new(),
            palettes_path: None,
            strict: false,
            env_allow: None,
            env_mask_logs: None,
            schema: false,
        };

        let report = validate_dir(config_dir, opts).expect("validate");

        // The "alt" palette is missing agent "build", so we expect a
        // "palette-mismatch" warning for the {{agent-build-model}} placeholder.
        let mismatch = report
            .findings
            .iter()
            .find(|f| f.kind == "palette-mismatch");
        assert!(
            mismatch.is_some(),
            "expected palette-mismatch finding but got: {:?}",
            report
                .findings
                .iter()
                .map(|f| format!("[{}] {}: {}", f.severity.as_label(), f.kind, f.message))
                .collect::<Vec<_>>()
        );

        let finding = mismatch.unwrap();
        assert_eq!(finding.severity, Severity::Warning);
        assert!(
            finding.message.contains("alt"),
            "finding should mention the palette missing the agent"
        );
    }
}
