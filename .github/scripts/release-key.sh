#!/usr/bin/env bash
# Write the release key's private half (RELEASE_GPG_KEY) to
# $RUNNER_TEMP/release-key.asc for the signing steps, after checking it is
# the key committed at packaging/linux/ayatsuri2d-signing-key.asc. A key
# from anyone else fails the run; no key at all leaves the files unsigned.
set -euo pipefail
PUBLIC=packaging/linux/ayatsuri2d-signing-key.asc
if [ -z "${RELEASE_GPG_KEY:-}" ]; then
  echo "::warning::RELEASE_GPG_KEY is not set: the .rpm and SHA256SUMS.txt go out unsigned"
  exit 0
fi
[ -f "$PUBLIC" ] || { echo "::error::$PUBLIC is missing, so the key can't be checked"; exit 1; }
fpr() { gpg --batch --with-colons --show-keys "$1" | awk -F: '/^fpr:/{print $10; exit}'; }
KEY="$RUNNER_TEMP/release-key.asc"
( umask 077; printf '%s\n' "$RELEASE_GPG_KEY" >"$KEY" )
WANT=$(fpr "$PUBLIC")
GOT=$(fpr "$KEY")
if [ -z "$GOT" ] || [ "$GOT" != "$WANT" ]; then
  rm -f "$KEY"
  echo "::error::the key in RELEASE_GPG_KEY is not the committed release key ($WANT)"
  exit 1
fi
echo "release key $WANT ready"
