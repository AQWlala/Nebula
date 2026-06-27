#!/usr/bin/env bash
# v1.0: one-line installer.
#
# Usage:
#   curl -fsSL https://nine-snake.app/install.sh | sh
#   ./scripts/install.sh                       # install latest release
#   ./scripts/install.sh --version=1.0.0      # pin a specific version
#   ./scripts/install.sh --dry-run            # print actions, do nothing
#   ./scripts/install.sh --local=./out        # install from a local build
#   ./scripts/install.sh --no-install         # just download the bundle
#
# P0#13 fix: previous version fabricated a `nine-snake-${target}.tar.gz`
# URL but `tauri build` actually produces platform-native installers
# (.msi / .dmg / .deb / .AppImage / .exe).  This rewrite maps the
# current OS+arch combo onto the real artifact names published to
# the GitHub release.
#
# Behaviour:
#   1. Detect OS + arch.
#   2. Pick the matching bundle file.
#   3. Download (or copy from --local) with a timeout + retry.
#   4. Install using the platform's native package manager, or
#      hand off to the user for Windows / macOS.

set -euo pipefail

DRY_RUN=0
NO_INSTALL=0
LOCAL_BUNDLE_DIR=""
VERSION="${NINE_SNAKE_VERSION:-1.0.0}"
TIMEOUT="${NINE_SNAKE_INSTALL_TIMEOUT:-120}"

REPO="${NINE_SNAKE_REPO:-nine-snake/nine-snake}"

usage() {
  sed -n '2,28p' "$0" | sed 's/^# \{0,1\}//'
  exit 0
}

for arg in "$@"; do
  case "$arg" in
    --dry-run)      DRY_RUN=1 ;;
    --no-install)   NO_INSTALL=1 ;;
    --version=*)    VERSION="${arg#--version=}" ;;
    --local=*)      LOCAL_BUNDLE_DIR="${arg#--local=}" ;;
    --timeout=*)    TIMEOUT="${arg#--timeout=}" ;;
    --repo=*)       REPO="${arg#--repo=}" ;;
    --help|-h)      usage ;;
    *) echo "unknown argument: $arg" >&2; exit 2 ;;
  esac
done

# -- 1. detect platform -------------------------------------------------
uname_s="$(uname -s 2>/dev/null || echo unknown)"
uname_m="$(uname -m 2>/dev/null || echo unknown)"

case "$uname_s" in
  Linux)   OS="linux" ;;
  Darwin)  OS="darwin" ;;
  MINGW*|MSYS*|CYGWIN*) OS="windows" ;;
  *)
    echo "unsupported OS: $uname_s" >&2
    exit 1
    ;;
esac

case "$uname_m" in
  x86_64|amd64)  ARCH="x86_64" ;;
  arm64|aarch64) ARCH="aarch64" ;;
  *)
    echo "unsupported arch: $uname_m" >&2
    exit 1
    ;;
esac

# -- 2. pick the bundle -------------------------------------------------
FILE=""
INSTALL_CMD=()

case "${OS}-${ARCH}" in
  linux-x86_64)
    FILE="nine-snake_${VERSION}_amd64.deb"
    INSTALL_CMD=(sudo dpkg -i)
    ;;
  linux-aarch64)
    FILE="nine-snake_${VERSION}_arm64.deb"
    INSTALL_CMD=(sudo dpkg -i)
    ;;
  darwin-x86_64)
    FILE="nine-snake-${VERSION}-x64.dmg"
    # .dmg needs an interactive handoff on macOS.
    INSTALL_CMD=()
    ;;
  darwin-aarch64)
    FILE="nine-snake-${VERSION}-aarch64.dmg"
    INSTALL_CMD=()
    ;;
  windows-x86_64)
    FILE="nine-snake_${VERSION}_x64-setup.exe"
    INSTALL_CMD=()
    ;;
  *)
    echo "no bundle published for ${OS}-${ARCH}" >&2
    exit 1
    ;;
esac

URL="https://github.com/${REPO}/releases/download/v${VERSION}/${FILE}"
DEST="/tmp/${FILE}"

echo "==> nine-snake v${VERSION} installer"
echo "    platform: ${OS}-${ARCH}"
echo "    bundle:   ${FILE}"

# -- 3. acquire the bundle ----------------------------------------------
if [[ -n "$LOCAL_BUNDLE_DIR" ]]; then
  echo "    using local bundle from ${LOCAL_BUNDLE_DIR}"
  if [[ ! -f "${LOCAL_BUNDLE_DIR}/${FILE}" ]]; then
    echo "    error: ${LOCAL_BUNDLE_DIR}/${FILE} not found" >&2
    exit 1
  fi
  DEST="${LOCAL_BUNDLE_DIR}/${FILE}"
else
  echo "    downloading ${URL}"
  if (( DRY_RUN )); then
    echo "    (dry-run, not actually downloading)"
  else
    # `curl --fail` makes a 404 hard-error.  We retry once on
    # transient network errors before giving up.
    ok=0
    for attempt in 1 2; do
      if curl --fail --silent --show-error --location \
             --connect-timeout 15 --max-time "${TIMEOUT}" \
             -o "${DEST}.partial" "${URL}"; then
        mv "${DEST}.partial" "${DEST}"
        ok=1
        break
      fi
      echo "    download failed (attempt ${attempt}/2), retrying…" >&2
      sleep 2
    done
    if (( ok == 0 )); then
      echo "    error: failed to download ${URL}" >&2
      rm -f "${DEST}.partial"
      exit 1
    fi
  fi
fi

if (( NO_INSTALL )) || (( DRY_RUN )); then
  echo "    bundle ready at: ${DEST}"
  if (( DRY_RUN )); then
    echo "    (dry-run, not installing)"
  else
    echo "    --no-install set, skipping install step"
  fi
  exit 0
fi

# -- 4. install / hand off ----------------------------------------------
case "${FILE}" in
  *.deb)
    if (( ${#INSTALL_CMD[@]} == 0 )); then
      echo "    error: no installer for ${FILE}" >&2
      exit 1
    fi
    "${INSTALL_CMD[@]}" "${DEST}"
    echo "    installed via dpkg"
    ;;
  *.dmg)
    echo "    opening ${DEST} — drag nine-snake.app into /Applications"
    open "${DEST}"
    ;;
  *.exe)
    echo "    launching ${DEST} (Windows installer)"
    if command -v cygstart >/dev/null 2>&1; then
      cygstart "${DEST}"
    elif command -v start >/dev/null 2>&1; then
      start "" "${DEST}"
    else
      echo "    please run ${DEST} manually" >&2
      exit 1
    fi
    ;;
  *)
    echo "    unknown bundle type, saved to: ${DEST}"
    echo "    please install manually"
    ;;
esac

echo
echo "==> done.  nine-snake v${VERSION} installed."
