# Examples

This directory contains a minimal working configuration and template that can
be used as a reference and as integration test fixtures.

## Layout

```text
examples/
  model-configs.yaml
  template.d/
    default.json
```

## Try it safely

```sh
repo_root="/path/to/opencode-config"
work_dir="$(mktemp -d)"
config_dir="$(mktemp -d)"
cp -R "$repo_root/examples/"* "$config_dir/"

(cd "$work_dir" && "$repo_root/target/debug/opencode-config" create default default \
  --config "$config_dir" \
  --out opencode.json)
```

Then inspect the output:

```sh
cat "$work_dir/opencode.json"
```
