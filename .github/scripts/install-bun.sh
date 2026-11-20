#!/usr/bin/env bash
# Installs Bun for the release jobs, verified before it ever runs. Release jobs hold the write token or the updater
# signing key, and oven-sh/setup-bun neither verifies its download nor waits before executing it. Both SHA-256
# values come from the official bun-v1.4.2 release: SHASUMS256.txt.asc, PGP signature verified with Bun's release key
# F3DCC08A8572C0749B3E18888EAB4D40A7B22B59. The zip is checked before it is unpacked, the binary before it is put on
# PATH. Bumping BUN_VERSION means re-deriving both hashes; see docs/release-and-updater-keys.md.
set -euo pipefail

readonly PINNED_VERSION="1.4.2"

if [[ "${BUN_VERSION:-}" != "$PINNED_VERSION" ]]; then
  echo "::error::BUN_VERSION is '${BUN_VERSION:-}', but install-bun.sh pins hashes for $PINNED_VERSION. Update both together."
  exit 1
fi

case "${RUNNER_OS:-}-${RUNNER_ARCH:-}" in
  Linux-X64)
    asset="bun-linux-x64"
    zip_sha="36368faef7527875d5ffa52e53cd48021741f2a83eb6208a8dd64068d422a913"
    bin_sha="a83d263767d839e4d2649ca8e35d07159c7afc99afdc96d731ced29e056dda0c"
    binary="bun"
    ;;
  macOS-ARM64)
    asset="bun-darwin-aarch64"
    zip_sha="90987a3a16d7db556d886ac3d551e7b6d3edf0a1cf43acaed622e8676be1d12f"
    bin_sha="35d20dd0263e5c950194434b925454fdfa9ba6e4467da960410fa05b08a7a5b5"
    binary="bun"
    ;;
  Windows-X64)
    asset="bun-windows-x64"
    zip_sha="ce4c17497b2f29712a99d3d53f028de28cd42e3bacb8589599e7f000e49b6405"
    bin_sha="15277c59ccd6c6c20f8dc9716c2b59c1776320d606b6a8658f70be8799519ca4"
    binary="bun.exe"
    ;;
  *)
    echo "::error::no pinned Bun build for ${RUNNER_OS:-?}-${RUNNER_ARCH:-?}"
    exit 1
    ;;
esac

# Hashes stdin, not the path: GNU sha256sum escapes its output (a leading "\") for paths with backslashes, which
# every Windows runner path has.
sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum < "$1" | cut -d ' ' -f 1
  else
    shasum -a 256 < "$1" | cut -d ' ' -f 1
  fi
}

expect_sha() {
  local file="$1" expected="$2" actual
  actual="$(sha256 "$file")"

  if [[ "$actual" != "$expected" ]]; then
    echo "::error::$file has SHA-256 $actual, expected $expected. Refusing to continue."
    exit 1
  fi
}

dir="$RUNNER_TEMP/bun-$PINNED_VERSION"
mkdir -p "$dir"

curl -fsSL --retry 3 -o "$dir/$asset.zip" \
  "https://github.com/oven-sh/bun/releases/download/bun-v$PINNED_VERSION/$asset.zip"
expect_sha "$dir/$asset.zip" "$zip_sha"

if command -v unzip >/dev/null 2>&1; then
  unzip -q -o "$dir/$asset.zip" -d "$dir"
else
  # Windows runners: Git Bash may lack unzip; 7-Zip is preinstalled there.
  7z x -y -bd -o"$dir" "$dir/$asset.zip" >/dev/null
fi

expect_sha "$dir/$asset/$binary" "$bin_sha"
chmod +x "$dir/$asset/$binary"

bin_dir="$dir/$asset"
if [[ "${RUNNER_OS}" == "Windows" ]]; then
  bin_dir="$(cygpath -w "$bin_dir")"
fi
echo "$bin_dir" >> "$GITHUB_PATH"

echo "Bun $PINNED_VERSION verified (${RUNNER_OS}-${RUNNER_ARCH})"
