#!/bin/sh

set -eu

script_name="$(basename -- "$0")"
script_dir=""
case "$script_name" in
  install.sh | install-open-interpreter.sh)
    script_dir="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
    ;;
esac

export CODEX_GITHUB_REPO="${OPEN_INTERPRETER_GITHUB_REPO:-${CODEX_GITHUB_REPO:-KillianLucas/oix}}"
export CODEX_INSTALL_PRODUCT_NAME="${CODEX_INSTALL_PRODUCT_NAME:-Open Interpreter}"
export CODEX_PACKAGE_ASSET_STEM="${CODEX_PACKAGE_ASSET_STEM:-open-interpreter-package}"
export CODEX_COMMAND_NAME="${CODEX_COMMAND_NAME:-interpreter}"
export CODEX_RELEASE_TAG_PREFIX="${CODEX_RELEASE_TAG_PREFIX:-rust-v}"
export CODEX_HOME="${OPEN_INTERPRETER_HOME:-${CODEX_HOME:-$HOME/.openinterpreter}}"
export CODEX_INSTALL_DIR="${OPEN_INTERPRETER_INSTALL_DIR:-${CODEX_INSTALL_DIR:-$HOME/.local/bin}}"
export CODEX_RELEASE="${OPEN_INTERPRETER_RELEASE:-${CODEX_RELEASE:-latest}}"
export CODEX_NON_INTERACTIVE="${OPEN_INTERPRETER_NONINTERACTIVE:-${CODEX_NON_INTERACTIVE:-false}}"

if [ -n "$script_dir" ] && [ -f "$script_dir/install-codex.sh" ]; then
  exec "$script_dir/install-codex.sh" "$@"
fi

if [ -n "$script_dir" ] && [ "$script_name" != "install.sh" ] && [ -f "$script_dir/install.sh" ]; then
  exec "$script_dir/install.sh" "$@"
fi

if command -v curl >/dev/null 2>&1; then
  curl -fsSL "https://github.com/$CODEX_GITHUB_REPO/releases/latest/download/install-codex.sh" | sh -s -- "$@"
  exit $?
fi

if command -v wget >/dev/null 2>&1; then
  wget -q -O - "https://github.com/$CODEX_GITHUB_REPO/releases/latest/download/install-codex.sh" | sh -s -- "$@"
  exit $?
fi

echo "curl or wget is required to install Open Interpreter." >&2
exit 1
