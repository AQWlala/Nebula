#!/usr/bin/env bash
# v1.0: build nine-snake for every supported target.
#
# Usage:
#   ./scripts/build-all.sh                  # build for the host
#   ./scripts/build-all.sh --all            # build for every target we ship
#   ./scripts/build-all.sh --targets <list> # space-separated target list
#
# Requirements:
#   - Rust toolchain (1.75+)
#   - Node 20+ and npm
#   - For cross-compilation: the matching rustup target installed.
#
# This script intentionally fails fast — it does not silently
# install missing tools.  Run `rustup target add <target>` first.

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$PROJECT_ROOT"

ALL_TARGETS=(
  "x86_64-unknown-linux-gnu"
  "aarch64-unknown-linux-gnu"
  "x86_64-apple-darwin"
  "aarch64-apple-darwin"
  "x86_64-pc-windows-msvc"
)

if [[ "${1:-}" == "--all" ]]; then
  TARGETS=("${ALL_TARGETS[@]}")
elif [[ "${1:-}" == "--targets" ]]; then
  shift
  TARGETS=("$@")
else
  # host target only
  TARGETS=("$(rustc -vV | sed -n 's|host: ||p')")
fi

echo "==> nine-snake cross-build"
echo "    targets: ${TARGETS[*]}"
echo

if ! command -v node >/dev/null; then
  echo "error: node is required (20+)" >&2
  exit 1
fi

if ! command -v cargo >/dev/null; then
  echo "error: cargo is required" >&2
  exit 1
fi

# v1.0 P0#10: regenerate the Tauri bundle icons if the asset
# directory is missing. The script is idempotent — running it a
# second time produces byte-identical files.
if [[ ! -f "src-tauri/icons/32x32.png" || ! -f "src-tauri/icons/icon.ico" ]]; then
  echo "==> generating Tauri bundle icons"
  if command -v python >/dev/null && python -c "import PIL" 2>/dev/null; then
    python scripts/generate-icons.py
  elif command -v python3 >/dev/null && python3 -c "import PIL" 2>/dev/null; then
    python3 scripts/generate-icons.py
  else
    echo "warning: Pillow not available; skipping icon generation. Install with 'pip install pillow' before running tauri:build." >&2
  fi
fi

if [[ ! -d node_modules ]]; then
  echo "==> npm ci"
  npm ci
fi

for target in "${TARGETS[@]}"; do
  echo "==> building for $target"
  if ! rustup target list --installed | grep -qx "$target"; then
    echo "    installing target $target"
    rustup target add "$target"
  fi
  npm run tauri:build -- --target "$target"
done

echo
echo "==> done"
