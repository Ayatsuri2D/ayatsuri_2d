#!/usr/bin/env bash
# Make the Ubuntu PPA source package for one release and one series:
#
#   ayatsuri2d_<version>.orig.tar.gz          the release tree (git archive of v<version>)
#   ayatsuri2d_<version>.orig-vendor.tar.xz   the vendored crates, unpacked to ./vendor
#   ayatsuri2d_<version>-0ppa<N>~<series>1.dsc, .debian.tar.xz, _source.changes
#
# Usage: packaging/debian/build-source.sh [--series noble] [--ppa-rev 1] [--ref v<version>]
# Needs dpkg-dev (dpkg-buildpackage, dpkg-source) and the build dependencies
# in debian/control if you also want a local binary build (see the end).
#
# Signing and upload are the maintainers' step, with the PPA's key:
#   debsign -k <KEY ID> $DIST/deb-src/ayatsuri2d_<v>-0ppa1~noble1_source.changes
#   dput ppa:<launchpad team>/<ppa> $DIST/deb-src/ayatsuri2d_<v>-0ppa1~noble1_source.changes
set -euo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"

SERIES=noble
PPA_REV=1
REF=""
while [ $# -gt 0 ]; do
  case "$1" in
    --series) SERIES="$2"; shift 2 ;;
    --ppa-rev) PPA_REV="$2"; shift 2 ;;
    --ref) REF="$2"; shift 2 ;;
    -h | --help) sed -n '2,16p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

# the Debian version: 0.6.0-beta becomes 0.6.0~beta, so it sorts before 0.6.0
UPVER="${VERSION/-/\~}"
DEBVER="$UPVER-0ppa$PPA_REV~${SERIES}1"
OUT="$DIST/deb-src"
mkdir -p "$OUT"

"$ROOT/packaging/vendor.sh" ${REF:+--ref "$REF"}
cp "$DIST/ayatsuri2d-$VERSION.tar.gz" "$OUT/ayatsuri2d_$UPVER.orig.tar.gz"
cp "$DIST/ayatsuri2d-$VERSION-vendor.tar.xz" "$OUT/ayatsuri2d_$UPVER.orig-vendor.tar.xz"

TREE="$OUT/ayatsuri2d-$UPVER"
rm -rf "$TREE"
mkdir -p "$TREE"
tar -xzf "$OUT/ayatsuri2d_$UPVER.orig.tar.gz" -C "$TREE" --strip-components=1
mkdir -p "$TREE/vendor"
tar -xJf "$OUT/ayatsuri2d_$UPVER.orig-vendor.tar.xz" -C "$TREE/vendor" --strip-components=1
# debian/ comes from the packaging folder of the tree being built
cp -R "$TREE/packaging/debian" "$TREE/debian"
rm -f "$TREE/debian/build-source.sh" "$TREE/debian/README.md"

# top changelog entry for this series and revision
DATE="$(date -R -u)"
{
  echo "ayatsuri2d ($DEBVER) $SERIES; urgency=medium"
  echo
  echo "  * Ayatsuri2D $VERSION for Ubuntu $SERIES."
  echo
  echo " -- Ayatsuri2D <support@ayatsuri-2d.org>  $DATE"
  echo
  cat "$TREE/debian/changelog"
} >"$TREE/debian/changelog.new"
mv "$TREE/debian/changelog.new" "$TREE/debian/changelog"

# source only, unsigned (-us -uc); -sa uploads the orig tarballs with it
(cd "$TREE" && dpkg-buildpackage -S -sa -d -us -uc)
echo "source package in $OUT (sign with debsign, upload with dput)"
echo "local test build: cd $TREE && dpkg-buildpackage -b -us -uc"
