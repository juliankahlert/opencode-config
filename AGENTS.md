AGENTS: Guidance for automated coding agents working in this repository

Purpose
- This file contains practical build/test/lint commands and a concise coding
  style guide that agentic coding agents should follow when editing the
  `opencode-config` repository.
- Agents must treat this repo as a Rust CLI project and follow the safety
  rules below before executing any commands that read or write user config.

Quick safety rules
- NEVER run the `opencode-config` binary in the repository root. Running the
  tool there can read `~/.config/opencode-config.d` and write `opencode.json`
  into this repository, potentially overwriting important files.
- When running the CLI for tests or development, always run from a temporary
  working directory and/or pass a config dir explicitly, e.g.:
  - `mkdir -p /tmp/opencode-test && cd /tmp/opencode-test`
  - `XDG_CONFIG_HOME=/tmp/opencode-config-config /path/to/target/debug/opencode-config create default github --config /tmp/opencode-config-config`

Build, lint, and test commands
- Build debug binary: `cargo build`
- Build release binary: `cargo build --release`
- Strip release binary: `strip -s target/release/opencode-config`
- Format code (apply): `cargo fmt --all`
- Format check (CI): `cargo fmt --all -- --check`
- Lint with Clippy: `cargo clippy --all-targets -- -D warnings`
- Run the whole test suite: `cargo test`
- Run a single unit test (example):
  - `cargo test some_test_name -- --exact --nocapture`
    - `--exact` forces an exact test-name match; `--nocapture` shows `println!`
      output.
  - You can also run by module/path: `cargo test module::submodule::test_name -- --exact`
- Run a single integration test (tests in `tests/`):
  - `cargo test --test integration_create test_case_name -- --exact --nocapture`
- Run bin-level command from build artifacts (safe usage):
  - create a temp working dir, set `XDG_CONFIG_HOME` to a temp dir and then run
    `target/debug/opencode-config ...` from that temp dir.
- Generate shell completions (if implemented):
  - `target/debug/opencode-config completions bash --out-dir ./completions`
- Generate RPM (developer tool): `cargo generate-rpm` (requires `cargo-generate-rpm`)

Developer toolchain recommendations
- Rust toolchain: `rustup` stable, match `edition = "2024"` in `Cargo.toml`.
- Optional developer tools: `cargo-audit`, `cargo-tarpaulin` (coverage),
  `cargo-watch` for iterative development.

Project structure conventions
- Binary entry: `src/main.rs` (must be minimal; delegate work to library code).
- Library root: `src/lib.rs` (public API used by `main.rs` and tests).
- Smaller modules: `src/<name>.rs` or `src/<name>/mod.rs` (file-per-module).
- Tests: unit tests live next to code in the module using `#[cfg(test)]`.
  Integration tests go into `tests/` and treat the binary as a black box.

Imports and ordering
- Group imports in this order: `std` first, external crates second, local
  crate imports (use `crate::` or `super::`) last.
- Avoid glob imports (`use foo::*;`). Be explicit to improve readability.
- Keep `use` lists short and import at the top of the file. When many
  related items are imported, prefer a grouped import: `use serde::{Deserialize, Serialize};`.

Formatting and style
- Use `rustfmt` defaults. Run `cargo fmt --all` before committing.
- Line width: follow `rustfmt` defaults (do not override locally in the repo).
- Prefer short functions (< 80–120 lines). Extract helpers for clarity.

Types, naming, and visibility
- Naming:
  - functions and variables: `snake_case`
  - types, structs, enums, and traits: `CamelCase`
  - constants and statics: `SCREAMING_SNAKE_CASE`
  - module files: `snake_case.rs` or `snake_case/mod.rs`
- Visibility:
  - Keep internal helper functions private. Only expose what is necessary in
    `lib.rs` for other modules and tests.
  - Use `pub(crate)` for crate-scoped items when tests or other modules need access.

Error handling
- Library code: prefer typed errors using `thiserror::Error` and return
  `Result<T, E>` where `E` is a domain error enum. This makes callers handle
  errors explicitly and keeps the library dependency-free of `anyhow`.
- Binary/CLI glue code: use `anyhow::Result` or `eyre` for convenience and add
  context with `.context("...")` for richer error messages.
- Avoid `unwrap()`/`expect()` in library code. Tests may use `unwrap()` but
  prefer `assert!` for expected invariants.
- Provide helpful user-facing error messages in CLI paths; include recovery
  suggestions where appropriate.

Logging and diagnostics
- Prefer `tracing` + `tracing-subscriber` for structured logging in the binary.
  Initialize the subscriber only in `main()`; do not initialize global
  subscribers from library code.
- For quick debugging in tests or small scripts, `eprintln!` is acceptable but
  prefer structured logs for production-quality CLI apps.

Serialization and config
- Use `serde` derives for config and template types. Keep `#[serde(default)]`
  where it makes sense and consider `#[serde(deny_unknown_fields)]` for
  config structs that should reject stray keys.

Template & placeholder handling (design notes for implementers)
- Template JSON files live under the config directory, typically
  `~/.config/opencode-config.d/template.d/<name>.json`.
- Placeholders use the form `{{agent-<name>-<field>}}` (example: `{{agent-build-model}}`).
- Replacement algorithm should:
  - parse template into `serde_json::Value`
  - build a `HashMap<String, String>` mapping placeholder keys to values
  - walk the JSON and replace placeholders in strings using a lightweight
    regex `\{\{\s*([^\}]+?)\s*\}\}`
  - if an entire string equals a missing `variant` placeholder, remove the
    `variant` key (unless `--strict` is used, in which case error)

Tests and CI
- Unit tests: colocate them with modules. Run: `cargo test`.
- Integration tests: place in `tests/` and use `assert_cmd` + `predicates` for
  robust CLI assertions. Example test workflow:
  - create temp config dir (use `tempfile` crate)
  - write `model-configs.yaml` and template files into that directory
  - `Command::cargo_bin("opencode-config")` to execute the binary with
    `--config` pointing to the temp config dir
  - assert filesystem changes in a temp working dir
- CI steps (recommended):
  1. `cargo fmt --all -- --check`
  2. `cargo clippy --all-targets -- -D warnings`
  3. `cargo test --workspace`

Commit messages and PRs
- Use Linux kernel style commit messages for atomic commits:
  - Subject line: `<area>: short description` (<= 50 chars)
  - Blank line
  - Body: describe why the change was made and any important details.
  - Wrap body at ~72 chars per line.
  - Example:
    - `create: add template loader and mapping builder`
    - ``
    - `Add load_template() which reads JSON templates into serde_json::Value.`
    - `This is the first step toward supporting template-based generation.`

AI / assistant-specific rules
- Do not modify or access user files outside the repository without explicit
  instruction. In particular, do not read or write `~/.config/opencode-config.d`.
- Prefer making small, well-tested changes as a single commit. Each change
  should include a test or a clear manual verification step.
- If a change touches config discovery or writes files, include a safety note
  and ensure tests run in a tempdir.

Cursor / Copilot rules
- This repository does not include `.cursor/rules/`, `.cursorrules`, or
  `.github/copilot-instructions.md`. If such files are added, agents must
  incorporate them into their behavior and follow their directives.

Contact and escalation
- If you are uncertain about a design choice, open a draft PR with a clear
  description and include tests that demonstrate the intended behavior.

End of AGENTS guidance
