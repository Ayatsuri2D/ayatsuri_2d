#!/usr/bin/env bash
# Regenerate packaging/flatpak/cargo-sources.json from Cargo.lock with
# flatpak-cargo-generator (flatpak-builder-tools), so the Flatpak build can
# fetch every crate as a checked source and build offline. Run it whenever
# Cargo.lock changes, and commit the result with the manifest.
#
# Usage: packaging/flatpak/gen-cargo-sources.sh [Cargo.lock]
# The generator is fetched at a pinned commit; set FPBT_COMMIT to move it.
# It needs Python 3.9+ with aiohttp and tomlkit: when those are missing, a
# throwaway virtualenv in the temp folder gets them from PyPI.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
LOCK="${1:-$ROOT/Cargo.lock}"
FPBT_COMMIT="${FPBT_COMMIT:-9d86d336a39b9af38b86f114b6ce5820fe260f91}"
URL="https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/$FPBT_COMMIT/cargo/flatpak-cargo-generator.py"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
curl -fsSL -o "$TMP/flatpak-cargo-generator.py" "$URL"

PY=python3
if ! "$PY" -c 'import aiohttp, tomlkit' 2>/dev/null; then
  echo "note: aiohttp/tomlkit missing, installing them into a temporary virtualenv"
  python3 -m venv "$TMP/venv"
  "$TMP/venv/bin/pip" install --quiet aiohttp tomlkit
  PY="$TMP/venv/bin/python"
fi

"$PY" "$TMP/flatpak-cargo-generator.py" "$LOCK" -o "$HERE/cargo-sources.json"
echo "wrote $HERE/cargo-sources.json ($(grep -c '"type": "archive"' "$HERE/cargo-sources.json") crates)"
