# opencode-config

A Rust CLI tool that generates `opencode.json` configuration files by combining
**model palettes** (YAML) with **JSON/YAML templates** and placeholder
substitution.

## Project Layout

```text
opencode-config/
  src/              # Library and binary source (Rust, edition 2024)
  tests/            # Integration tests (assert_cmd + predicates)
  examples/         # Minimal working config and template examples
  pages/            # mdbook documentation site (deployed via GitHub Pages)
  .github/workflows # CI (ci.yml) and documentation deploy (pages.yml)
  AGENTS.md         # Guidance for automated coding agents
  LICENSE           # MIT
```

Key source modules: `cli.rs` (argument parsing), `config.rs` (config
discovery), `template.rs` / `substitute.rs` (placeholder engine),
`palette_io.rs` (YAML palette loading), `create.rs` (output generation),
`render.rs` (rendering pipeline), `validate.rs` (schema validation).

## Build & Test

```sh
cargo build                                   # debug build
cargo build --release && strip -s target/release/opencode-config  # release
cargo fmt --all                               # format
cargo clippy --all-targets -- -D warnings     # lint
cargo test                                    # full test suite
cargo test some_test -- --exact --nocapture   # single test with output
```

> **Safety:** never run the binary from the repository root -- it can write
> `opencode.json` into the repo. Always use a temporary working directory and
> pass `--config` or set `XDG_CONFIG_HOME` explicitly. See `AGENTS.md` for
> details.

## Usage Overview

```text
opencode-config create  <template> <palette> [--out <file>] [--force] [--config <dir>]
opencode-config switch  <template> <palette> [--out <file>] [--config <dir>]
opencode-config list-templates  [--config <dir>]
opencode-config list-palettes   [--config <dir>]
opencode-config completions <shell> --out-dir <dir>
```

| Command | Purpose |
|---|---|
| `create` | Generate `opencode.json` from a template + palette |
| `switch` | Like `create` but always overwrites (implies `--force`) |
| `list-templates` | Show available templates |
| `list-palettes` | Show available model palettes |
| `completions` | Generate shell completions (bash, zsh, fish, elvish, PowerShell) |

Global flags `--strict` / `--no-strict` control whether missing placeholders
cause errors or are silently removed.

## Configuration

The tool reads from `~/.config/opencode-config.d` (or `$XDG_CONFIG_HOME/opencode-config.d`),
overridable with `--config <dir>`:

```text
opencode-config.d/
  model-configs.yaml      # palette definitions (agents + models)
  config.yaml             # optional run options (strict, env_allow, ...)
  template.d/
    default.json          # one or more JSON/YAML templates
```

### Templates & Placeholders

Templates contain placeholders of the form `{{agent-<name>-<field>}}` (e.g.
`{{agent-build-model}}`). The engine walks the JSON tree, matches placeholders
via regex, and substitutes values from the selected palette. Special rules:

- **Variant removal** -- missing `variant` placeholders remove the key entirely
  (unless `--strict`).
- **Node-level substitution** -- a string value that is exactly one placeholder
  is replaced with the resolved value's native type (number, bool, object).
- **Alias shorthand** -- `"model": "{{build}}"` copies model/variant/reasoning
  from the named palette agent before regular substitution.
- **YAML templates** -- `.yaml`/`.yml` templates are converted to JSON before
  substitution; anchors and comments are not preserved.

## Contributing

1. Fork and create a feature branch.
2. Follow the style in `AGENTS.md` -- `cargo fmt`, `cargo clippy`, colocated
   unit tests, integration tests in `tests/`.
3. Use Linux kernel-style commit messages: `<area>: short description`.
4. Open a PR with tests that demonstrate the new behavior.

See [AGENTS.md](AGENTS.md) for full build commands, safety rules, and coding
conventions.

## Documentation

An mdbook site is published from the `pages/` directory via GitHub Pages. Build
locally:

```sh
cd pages && mdbook serve
```

## License

[MIT](LICENSE) -- Copyright (c) 2026 Julian Kahlert
