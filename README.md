# opencode-config

<!-- [![CI](https://github.com/anomalyco/opencode-config/actions/workflows/ci.yml/badge.svg)](https://github.com/anomalyco/opencode-config/actions/workflows/ci.yml) -->
<!-- [![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE) -->

A Rust CLI tool that generates `opencode.json` configuration files by combining
**model palettes** (YAML) with **JSON/YAML templates** and placeholder
substitution.

---

## Project Layout

```text
opencode-config/
  src/              # Library and binary source (Rust, edition 2024)
  tests/            # Integration tests (assert_cmd + predicates)
  examples/         # Minimal working config and template examples
  example-config/   # Additional example configuration and output
  completions/      # Pre-generated shell completions (bash, zsh, fish, elvish, PowerShell)
  pages/            # mdbook documentation site (deployed via GitHub Pages)
  .github/workflows # CI (ci.yml) and documentation deploy (pages.yml)
  AGENTS.md         # Guidance for automated coding agents
  LICENSE           # MIT
```

Key source modules: `cli.rs` (argument parsing), `config.rs` (config
discovery), `template.rs` / `substitute.rs` (placeholder engine),
`palette_io.rs` (YAML palette loading), `create.rs` (output generation),
`render.rs` (rendering pipeline), `validate.rs` (schema validation),
`compose.rs` / `decompose.rs` (fragment assembly), `schema.rs` (JSON Schema
generation), `wizard.rs` (interactive create wizard), `options.rs` (run-option
resolution), `diff.rs` (dry-run diff output).

---

## Build & Test

```sh
cargo build                                   # debug build
cargo build --release && strip -s target/release/opencode-config  # release
cargo fmt --all                               # format
cargo clippy --all-targets -- -D warnings     # lint
cargo test                                    # full test suite
cargo test some_test -- --exact --nocapture   # single test with output
```

> **Safety:** Never run the binary from the repository root -- it can read
> `~/.config/opencode-config.d` and write `opencode.json` into the repo.
> Always use a temporary working directory and pass `--config` or set
> `XDG_CONFIG_HOME` explicitly. See [AGENTS.md](AGENTS.md) for details.

---

## Usage Overview

```text
opencode-config [OPTIONS] <COMMAND>
```

### Commands

| Command | Purpose |
|---|---|
| `create` | Generate `opencode.json` from a template + palette |
| `switch` | Like `create` but always overwrites (implies `--force`) |
| `render` | Render a template + palette to stdout or a file (no config write) |
| `validate` | Validate templates and palettes (text or JSON report) |
| `list-templates` | Show available templates |
| `list-palettes` | Show available model palettes |
| `schema` | Generate JSON Schema artifacts |
| `palette` | Import or export palettes |
| `decompose` | Decompose a single-file template into per-section fragments |
| `compose` | Compose fragment files back into a single template or config |
| `completions` | Generate shell completions (bash, zsh, fish, elvish, PowerShell) |

### Global Flags

| Flag | Short | Description |
|---|---|---|
| `--strict` | `-S` | Enable strict mode (missing placeholders cause errors) |
| `--no-strict` | | Disable strict mode |
| `--config <DIR>` | | Override config directory path |
| `--env-allow` | | Allow `{{env:VAR}}` placeholders to resolve from the environment |
| `--no-env` | | Disable environment placeholder resolution |
| `--env-mask-logs` | | Mask resolved environment values in log output |
| `--no-env-mask-logs` | | Disable masking of environment values in logs |
| `--verbose` | `-v` | Enable debug-level logging |

### Command Details

**create** -- generate `opencode.json`:

```sh
opencode-config create <TEMPLATE> <PALETTE> [--out <FILE>] [--force] [--dry-run]
opencode-config create -i                    # interactive wizard
```

**switch** -- overwrite an existing `opencode.json`:

```sh
opencode-config switch <TEMPLATE> <PALETTE> [--out <FILE>] [--dry-run]
```

**render** -- render without writing config:

```sh
opencode-config render -t <TEMPLATE> -p <PALETTE> [-o <FILE>] [--format json|yaml] [--dry-run]
```

**validate** -- check templates and palettes:

```sh
opencode-config validate [--templates <GLOB>] [--palettes <FILE>] [--format text|json] [--schema]
```

**palette export / import** -- share palettes across machines:

```sh
opencode-config palette export --name <PALETTE> [-o <FILE>] [--format json|yaml]
opencode-config palette import --from <FILE> [--name <PALETTE>] [--merge abort|overwrite|merge] [--dry-run] [--force]
```

**decompose / compose** -- work with template fragments:

```sh
opencode-config decompose <TEMPLATE> [--dry-run] [--verify] [--force]
opencode-config compose [INPUT] [-o <FILE>] [--dry-run] [--verify] [--backup] [--pretty|--minify] [--conflict error|last-wins|interactive]
```

**schema generate** -- generate JSON Schema from a palette:

```sh
opencode-config schema generate --palette <PALETTE> [--out <DIR>]
```

**completions** -- generate shell completions:

```sh
opencode-config completions <bash|zsh|fish|elvish|power-shell> --out-dir <DIR>
```

---

## Configuration

The tool reads from `~/.config/opencode-config.d` (or
`$XDG_CONFIG_HOME/opencode-config.d`), overridable with `--config <DIR>`:

```text
opencode-config.d/
  model-configs.yaml      # palette definitions (agents + models)
  config.yaml             # optional run options (strict, env_allow, env_mask_logs)
  template.d/
    default.json          # one or more JSON/YAML templates
    default.d/            # or a fragment directory for the same template
```

### Palettes (`model-configs.yaml`)

Palettes define named sets of agent configurations:

```yaml
palettes:
  default:
    agents:
      build:
        model: openrouter/openai/gpt-4o
        variant: mini
        reasoning: true
      review:
        model: openrouter/openai/gpt-4o
        reasoning:
          effort: medium
          text_verbosity: low
    mapping:
      build-count: 3
      build-flags:
        - fast
        - safe
```

Each agent has a required `model` field and optional `variant` and `reasoning`
fields. The `mapping` section allows arbitrary key-value pairs for custom
placeholders.

### Run Options (`config.yaml`)

```yaml
strict: false
env_allow: false
env_mask_logs: false
```

Option resolution follows a strict precedence chain:
**CLI flags > config.yaml > environment (`OPENCODE_STRICT`) > defaults**.

### Templates & Placeholders

Templates contain placeholders of the form `{{agent-<name>-<field>}}` (e.g.
`{{agent-build-model}}`). The engine walks the JSON tree, matches placeholders
via regex `\{\{\s*([^\}]+?)\s*\}\}`, and substitutes values from the selected
palette. Special rules:

- **Variant removal** -- missing `variant` placeholders remove the key entirely
  (unless `--strict`).
- **Node-level substitution** -- a string value that is exactly one placeholder
  is replaced with the resolved value's native type (number, bool, object).
- **Alias shorthand** -- `"model": "{{build}}"` copies model/variant/reasoning
  from the named palette agent before regular substitution.
- **Environment placeholders** -- `{{env:VAR}}` resolves from the process
  environment when `--env-allow` is active.
- **YAML templates** -- `.yaml`/`.yml` templates are converted to JSON before
  substitution; anchors and comments are not preserved.
- **Directory templates** -- a `<name>.d/` directory is assembled by merging
  fragments in lexicographic order before rendering.

---

## Quick Start

```sh
# 1. Set up a config directory
config_dir="$(mktemp -d)"
cp -R examples/* "$config_dir/"

# 2. Generate opencode.json in a safe temp directory
work_dir="$(mktemp -d)"
cd "$work_dir"
/path/to/target/debug/opencode-config create default default \
  --config "$config_dir" --out opencode.json

# 3. Inspect output
cat opencode.json
```

See [`examples/README.md`](examples/README.md) for the full minimal example.

---

## Contributing

1. Fork and create a feature branch.
2. Follow the style in [AGENTS.md](AGENTS.md) -- `cargo fmt`, `cargo clippy`,
   colocated unit tests, integration tests in `tests/`.
3. Use Linux kernel-style commit messages: `<area>: short description`.
4. Open a PR with tests that demonstrate the new behavior.

See [AGENTS.md](AGENTS.md) for full build commands, safety rules, and coding
conventions.

---

## Documentation

An mdbook site is published from the `pages/` directory via GitHub Pages. Build
locally:

```sh
cd pages && mdbook serve
```

---

## License

[MIT](LICENSE) -- Copyright (c) 2026 Julian Kahlert
