# opencode-config

Generate `opencode.json` files from a palette of models and JSON/YAML templates.

## Install

- Build from source:

  ```sh
  cargo build
  # or
  cargo build --release
  ```

- Run directly from a debug build (from a temp working dir):

  ```sh
  repo_root="/path/to/opencode-config"
  work_dir="$(mktemp -d)"
  config_home="$(mktemp -d)"
  mkdir -p "$config_home/opencode-config.d"

  (cd "$work_dir" && XDG_CONFIG_HOME="$config_home" \
    "$repo_root/target/debug/opencode-config" --help)
  ```

## Usage

```sh
opencode-config [--strict|--no-strict] create <template> <palette> [--out <file>] [--force] \
  [--config <dir>]
opencode-config [--strict|--no-strict] switch <template> <palette> [--out <file>] \
  [--config <dir>]
opencode-config list-templates [--config <dir>]
opencode-config list-palettes [--config <dir>]
opencode-config completions <shell> --out-dir <dir>
```

Note: `switch` behaves like `create` but always overwrites the output file
(equivalent to `create --force`).

## Safety

`create` writes `opencode.json` to the current working directory by default.
**Never run the binary from this repository root.** Always use a temporary
working directory and pass an explicit config directory or set
`XDG_CONFIG_HOME` for every command.

Example (safe):

```sh
repo_root="/path/to/opencode-config"
work_dir="$(mktemp -d)"
config_dir="$(mktemp -d)"
cp -R "$repo_root/examples/"* "$config_dir/"

(cd "$work_dir" && "$repo_root/target/debug/opencode-config" create default default \
  --config "$config_dir" \
  --out opencode.json)
```

Using `XDG_CONFIG_HOME`:

```sh
repo_root="/path/to/opencode-config"
work_dir="$(mktemp -d)"
config_home="$(mktemp -d)"
mkdir -p "$config_home/opencode-config.d"
cp -R "$repo_root/examples/"* "$config_home/opencode-config.d/"

(cd "$work_dir" && XDG_CONFIG_HOME="$config_home" \
  "$repo_root/target/debug/opencode-config" list-templates)
```

## Configuration layout

The config directory contains a model palette file and a template directory:

```text
opencode-config.d/
  model-configs.yaml
  template.d/
    default.json
```

When you pass `--config`, provide the path that contains `model-configs.yaml`
and `template.d/`. Without `--config`, the tool uses
`$XDG_CONFIG_HOME/opencode-config.d` (usually `~/.config/opencode-config.d`).

### model-configs.yaml

```yaml
palettes:
  default:
    agents:
      build:
        model: openrouter/openai/gpt-4o
        variant: mini
```

Model palettes in `model-configs.yaml` use the `agents` key (plural), while
templates use `agent` (singular) for the JSON structure. Keep this distinction
when updating examples.

### Run options (config.yaml)

`config.yaml` is optional and can include other top-level keys. When present, it
must be a valid YAML mapping; empty files or `null`/`~` are treated as invalid by
the current parser. Run options are read from the top level:

```yaml
strict: false
env_allow: false
env_mask_logs: false
```

Strict precedence is CLI (`--strict`/`--no-strict`) > config.yaml >
`OPENCODE_STRICT` > default (false). Boolean values accept
`true/false`, `1/0`, `yes/no`, and `on/off`.

### Templates and placeholders

Template JSON/YAML files can include placeholders like:

- `{{agent-<name>-model}}`
- `{{agent-<name>-variant}}`
- `{{agent-<name>-reasoning-effort}}`
- `{{agent-<name>-text-verbosity}}`

Missing `variant` placeholders are removed only when the key is exactly
`"variant"` and the value is a full placeholder ending in `-variant`
(whitespace allowed). Otherwise the key/value is retained (non-strict) or
errors (strict).

Non-string placeholders support node-level substitution: when a string value is
exactly a placeholder (whitespace allowed) and the resolved value is non-string,
the entire JSON node is replaced with the resolved value. If a placeholder is
embedded inside a larger string, strict mode errors on non-string values;
permissive mode stringifies the value.

If an agent object's `model` field is exactly a placeholder (whitespace
allowed), e.g. `"model": "{{build}}"`, the tool treats it as an alias to the
palette agent `build` and copies its model/variant/reasoning before regular
placeholder substitution. This aliasing runs first. The recommended forms are
`{{agent-<name>-model}}`/`{{agent-<name>-variant}}`; the bare `{{<name>}}` forms
are legacy.

YAML templates (`.yaml`/`.yml`) are also supported. YAML templates are
converted to JSON before substitution, so anchors/comments are not preserved.
Non-string map keys are unsupported and will cause a conversion error (they are
not preserved). The rendered output remains JSON (until a `render` command
supports other formats).

Template names passed to `create` or `switch` are base names only. The resolver
checks `<name>.json`, then `<name>.yaml`, then `<name>.yml` under `template.d/`.
Explicit file paths are not supported.

Example:

```json
{
  "agent": {
    "build": { "model": "{{build}}" }
  }
}
```

## Examples

See [examples/README.md](examples/README.md) for a minimal working config and
template.
