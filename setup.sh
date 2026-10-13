#!/bin/sh
set -eu
cd "$(dirname "$0")"
. ./scripts/dev-tools.env

for prerequisite in git rustup uv just prek; do
    if ! command -v "$prerequisite" >/dev/null; then
        printf 'Install %s before running setup.\n' "$prerequisite" >&2
        exit 1
    fi
done

require_version() {
    expected=$1
    command=$2
    if ! "$command" --version 2>/dev/null | awk -v version="$expected" '
        { for (i = 1; i <= NF; i++) if ($i == version || index($i, version "-") == 1) found = 1 }
        END { exit !found }
    '; then
        printf 'Install %s %s before running setup.\n' "$command" "$expected" >&2
        exit 1
    fi
}

require_version "$UV_VERSION" uv
require_version "$JUST_VERSION" just
require_version "$PREK_VERSION" prek
rustup show active-toolchain
rustup component add rustfmt clippy llvm-tools-preview rust-analyzer
uv sync --locked
cargo fetch --locked
prek install-hooks
prek install --hook-type pre-commit --hook-type commit-msg --hook-type pre-push
printf 'Development environments are ready; run just check.\n'
