#!/usr/bin/env bash
# Print the SPDX License: expression for ayatsuri2d.spec: our own licences
# AND every distinct licence expression of the crates statically linked into
# the two Linux binaries (normal dependencies only, as Fedora's
# %cargo_license_summary counts them; build and test crates are left out).
#
# Usage: packaging/fedora/license-summary.sh [target triple]   (default: x86_64-unknown-linux-gnu)
# With --list it prints one "crate version: licence" line per crate instead.
#
# In a Fedora build, %cargo_license_summary gives the authoritative answer;
# run this when Cargo.lock changes so the spec's License: does not go stale.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
LIST=0
TARGET=x86_64-unknown-linux-gnu
for a in "$@"; do
  case "$a" in
    --list) LIST=1 ;;
    *) TARGET="$a" ;;
  esac
done

tree() {
  cargo tree --manifest-path "$ROOT/Cargo.toml" --locked --offline \
    -p ayatsuri -p ayatsuri-launcher -e normal --target "$TARGET" \
    --prefix none --format "$1"
}

# third-party crates only, each once: "name vX: licence"; our own crates
# (listed with their path) are covered by the first line of the result
crates() {
  tree '{p}|{l}' | sed 's/ (\*)$//' | grep -v -F "($ROOT" | sort -u
}

if [ "$LIST" = 1 ]; then
  crates | sed 's/|/: /'
  exit 0
fi

# Fedora spells the old slash form as OR, and wants each compound term
# parenthesised inside the AND list
{
  echo "GPL-3.0-only"
  echo "MIT"
  echo "Apache-2.0"
  crates | cut -d'|' -f2- | sed -e 's| */ *| OR |g' -e 's/^ *//' -e 's/ *$//' | grep -v '^$' |
    grep -v -x -e 'GPL-3.0-only' -e MIT -e Apache-2.0 | sort -u
} |
  awk '
    / (OR|AND|WITH) / && !/^\(.*\)$/ { $0 = "(" $0 ")" }
    { terms[NR] = $0 }
    END { for (i = 1; i <= NR; i++) printf "%s%s\n", terms[i], (i < NR ? " AND" : "") }
  '
