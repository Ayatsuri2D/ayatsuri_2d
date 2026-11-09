#!/usr/bin/env bash
# Write the vendored-crates tarball that offline source builds need (the
# Fedora/COPR spec and the Debian/Launchpad source package), next to the
# plain source tarball of the same version:
#
#   $DIST/ayatsuri2d-<version>.tar.gz          git archive of the tree, top folder ayatsuri_2d-<version>/
#   $DIST/ayatsuri2d-<version>-vendor.tar.xz   vendor/<crate>-<version>/..., from Cargo.lock
#
# Usage: packaging/vendor.sh [--ref <git ref>]   (default ref: v<version>, else HEAD)
#
# With cargo-vendor-filterer installed (cargo install cargo-vendor-filterer)
# only the crates Linux needs are kept, which makes the tarball much smaller;
# plain `cargo vendor` keeps the Windows and macOS crates too. Either builds.
set -euo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/env.sh"

REF=""
while [ $# -gt 0 ]; do
  case "$1" in
    --ref) REF="$2"; shift 2 ;;
    -h | --help) sed -n '2,13p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
if [ -z "$REF" ]; then
  if git -C "$ROOT" rev-parse -q --verify "refs/tags/v$VERSION" >/dev/null; then REF="v$VERSION"; else REF=HEAD; fi
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
SRC="$WORK/ayatsuri_2d-$VERSION"

echo "==> source tarball of $REF"
git -C "$ROOT" archive --format=tar --prefix="ayatsuri_2d-$VERSION/" "$REF" | tar -x -C "$WORK"
tar -C "$WORK" -czf "$DIST/ayatsuri2d-$VERSION.tar.gz" "ayatsuri_2d-$VERSION"

echo "==> vendoring the crates in Cargo.lock"
if command -v cargo-vendor-filterer >/dev/null; then
  (cd "$SRC" && cargo vendor-filterer --locked --versioned-dirs \
    --platform=x86_64-unknown-linux-gnu --platform=aarch64-unknown-linux-gnu vendor >/dev/null)
else
  echo "note: cargo-vendor-filterer not found, vendoring every platform's crates"
  (cd "$SRC" && cargo vendor --quiet --locked --versioned-dirs vendor >/dev/null)
fi
# fixed order, owner and times, so the same Cargo.lock gives the same tarball
tar -C "$SRC" --sort=name --owner=0 --group=0 --numeric-owner \
  --mtime="@$(git -C "$ROOT" log -1 --format=%ct "$REF")" \
  -cJf "$DIST/ayatsuri2d-$VERSION-vendor.tar.xz" vendor

echo "wrote $DIST/ayatsuri2d-$VERSION.tar.gz"
echo "wrote $DIST/ayatsuri2d-$VERSION-vendor.tar.xz ($(du -h "$DIST/ayatsuri2d-$VERSION-vendor.tar.xz" | cut -f1))"
