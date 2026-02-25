set shell := ["bash", "-euo", "pipefail", "-c"]

bin := "target/release/opencode-config"
rpm_dir := "target/generate-rpm"
rpm_link := "target/opencode-config-latest.rpm"

completions:
  #!/usr/bin/env bash
  set -euo pipefail
  cargo build --release
  repo_root="$(pwd)"
  work_dir="$(mktemp -d)"
  config_home="$(mktemp -d)"
  trap 'rm -rf "$work_dir" "$config_home"' EXIT
  mkdir -p "$repo_root/completions"
  cd "$work_dir"
  export XDG_CONFIG_HOME="$config_home"
  "$repo_root"/{{bin}} completions bash --out-dir "$repo_root/completions"
  "$repo_root"/{{bin}} completions zsh --out-dir "$repo_root/completions"
  "$repo_root"/{{bin}} completions fish --out-dir "$repo_root/completions"
  "$repo_root"/{{bin}} completions elvish --out-dir "$repo_root/completions"
  "$repo_root"/{{bin}} completions power-shell --out-dir "$repo_root/completions"

rpm: completions
  cargo generate-rpm
  ln -sf {{rpm_dir}}/*.rpm {{rpm_link}}

install:
  sudo dnf install {{rpm_link}}

reinstall:
  sudo dnf reinstall {{rpm_dir}}/*.rpm

pages:
  #!/usr/bin/env bash
  set -euo pipefail
  for cmd in mdbook mdbook-mermaid; do
    if ! command -v "$cmd" &>/dev/null; then
      echo "error: '$cmd' is not installed" >&2
      exit 1
    fi
  done
  mdbook build pages
