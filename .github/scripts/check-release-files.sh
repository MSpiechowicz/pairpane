#!/usr/bin/env bash
# Release workflow, job `check` (read-only): checks the signed release files in dist/ and writes latest.json and
# checksums.txt next to them. Runs from the verified release commit (the CI-tested commit plus the version bump).
#
#   VERSION=X.Y.Z GITHUB_REPOSITORY=owner/repo RUNNER_TEMP=/tmp .github/scripts/check-release-files.sh
set -euo pipefail

v="${VERSION:?}"
tmp="${RUNNER_TEMP:?}"
updater=("Pairpane_${v}_amd64.AppImage" "Pairpane_${v}_amd64.deb" "Pairpane-${v}-1.x86_64.rpm"
  "Pairpane_${v}_aarch64.app.tar.gz" "Pairpane_${v}_x64.app.tar.gz"
  "Pairpane_${v}_x64_en-US.msi" "Pairpane_${v}_x64-setup.exe")

# Exactly the expected installers, each updater artifact with a signature made by the committed public key for this
# file name and this version: a key mismatch, a tampered file or a replayed older signature fails here.
expected=("${updater[@]}" "Pairpane_${v}_aarch64.dmg" "Pairpane_${v}_x64.dmg")
for name in "${updater[@]}"; do expected+=("$name.sig"); done
if ! diff <(find dist -mindepth 1 -maxdepth 1 -printf '%f\n' | sort) <(printf '%s\n' "${expected[@]}" | sort); then
  echo "::error::the signed artifact does not contain exactly the expected files"
  exit 1
fi

sudo apt-get update
sudo apt-get install -y minisign
jq -r '.plugins.updater.pubkey' src-tauri/tauri.conf.json | base64 -d > "$tmp/updater.pub"
for name in "${updater[@]}"; do
  base64 -d < "dist/$name.sig" > "$tmp/current.minisig"
  output="$(minisign -V -m "dist/$name" -p "$tmp/updater.pub" -x "$tmp/current.minisig")"
  comment="$(sed -n 's/^Trusted comment: //p' <<< "$output" | tr '\t' '\n')"
  if ! grep -qxF "file:$name" <<< "$comment" || ! grep -qxF "version:$v" <<< "$comment"; then
    echo "::error::the signature of $name is not bound to file $name and version $v"
    exit 1
  fi
done
echo "${#updater[@]} signatures verified"

# latest.json: the updater manifest, one signed entry per platform and installer type.
base="https://github.com/${GITHUB_REPOSITORY:?}/releases/download/v$v"
entry() {
  jq -n --arg url "$base/$1" --rawfile signature "dist/$1.sig" \
    '{signature: ($signature | rtrimstr("\n")), url: $url}'
}
appimage="$(entry "Pairpane_${v}_amd64.AppImage")"
deb="$(entry "Pairpane_${v}_amd64.deb")"
rpm="$(entry "Pairpane-${v}-1.x86_64.rpm")"
mac_arm="$(entry "Pairpane_${v}_aarch64.app.tar.gz")"
mac_x64="$(entry "Pairpane_${v}_x64.app.tar.gz")"
msi="$(entry "Pairpane_${v}_x64_en-US.msi")"
nsis="$(entry "Pairpane_${v}_x64-setup.exe")"
jq -n --arg version "$v" --arg pub_date "$(date -u +%Y-%m-%dT%H:%M:%S.000Z)" \
  --argjson appimage "$appimage" --argjson deb "$deb" --argjson rpm "$rpm" \
  --argjson mac_arm "$mac_arm" --argjson mac_x64 "$mac_x64" --argjson msi "$msi" --argjson nsis "$nsis" \
  '{version: $version, notes: "", pub_date: $pub_date, platforms: {
    "linux-x86_64": $appimage, "linux-x86_64-appimage": $appimage,
    "linux-x86_64-deb": $deb, "linux-x86_64-rpm": $rpm,
    "darwin-aarch64": $mac_arm, "darwin-aarch64-app": $mac_arm,
    "darwin-x86_64": $mac_x64, "darwin-x86_64-app": $mac_x64,
    "windows-x86_64": $msi, "windows-x86_64-msi": $msi, "windows-x86_64-nsis": $nsis}}' > dist/latest.json

# checksums.txt: install.sh / install.ps1 install nothing without a SHA-256 to compare against.
(cd dist && sha256sum -- * > "$tmp/checksums.txt")
mv "$tmp/checksums.txt" dist/
