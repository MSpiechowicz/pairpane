#!/usr/bin/env bash
# Installs Node.js for the release job (semantic-release is a Node program and runs with the write token), verified
# before it ever runs. The SHA-256 comes from the official v26.13.1 SHASUMS256.txt.asc, PGP signature verified with
# release key 5BE8A3F6C8A5C01D106C0AD820B1A390B168D356 (listed as a Node.js releaser in the nodejs/node README).
# Bumping NODE_VERSION means re-deriving the hash; see docs/release-and-updater-keys.md.
set -euo pipefail

readonly PINNED_VERSION="26.13.1"
readonly ARCHIVE_SHA="5c063e46bab0fbc1c17cc445d7ff129cbdcd0471be6ed83b6ab864f8143bc864"

if [[ "${NODE_VERSION:-}" != "$PINNED_VERSION" ]]; then
  echo "::error::NODE_VERSION is '${NODE_VERSION:-}', but install-node.sh pins the hash for $PINNED_VERSION. Update both together."
  exit 1
fi

if [[ "${RUNNER_OS:-}-${RUNNER_ARCH:-}" != "Linux-X64" ]]; then
  echo "::error::install-node.sh only pins the Linux x64 build (got ${RUNNER_OS:-?}-${RUNNER_ARCH:-?})"
  exit 1
fi

name="node-v$PINNED_VERSION-linux-x64"
dir="$RUNNER_TEMP/node-$PINNED_VERSION"
mkdir -p "$dir"

curl -fsSL --retry 3 -o "$dir/$name.tar.xz" "https://nodejs.org/dist/v$PINNED_VERSION/$name.tar.xz"

actual="$(sha256sum "$dir/$name.tar.xz" | cut -d ' ' -f 1)"
if [[ "$actual" != "$ARCHIVE_SHA" ]]; then
  echo "::error::$name.tar.xz has SHA-256 $actual, expected $ARCHIVE_SHA. Refusing to continue."
  exit 1
fi

tar -xJf "$dir/$name.tar.xz" -C "$dir"
echo "$dir/$name/bin" >> "$GITHUB_PATH"

echo "Node.js $PINNED_VERSION verified (Linux-X64)"
