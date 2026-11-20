#!/bin/sh
# Pairpane installer for macOS and Linux.
#
#   curl --proto '=https' --tlsv1.2 -fsSL https://raw.githubusercontent.com/MSpiechowicz/pairpane/main/install.sh | sh
#
# Linux gets the AppImage as ~/.local/bin/pairpane plus an applications-menu entry, macOS gets
# ~/Applications/Pairpane.app. Both are the files the in-app updater replaces in place.
#
# Environment overrides:
#   PAIRPANE_VERSION      release tag to install (default: latest), e.g. v0.2.0
#   PAIRPANE_INSTALL_DIR  where the Linux AppImage goes (default: ~/.local/bin)
#   PAIRPANE_NO_SHORTCUT  set to 1 to skip the Linux applications-menu entry
set -eu

REPO="MSpiechowicz/pairpane"
BIN="pairpane"
APP_NAME="Pairpane"
INSTALL_DIR="${PAIRPANE_INSTALL_DIR:-$HOME/.local/bin}"
case "$INSTALL_DIR" in
  /*) ;;
  *) printf 'error: PAIRPANE_INSTALL_DIR must be an absolute path (got %s)\n' "$INSTALL_DIR" >&2; exit 1 ;;
esac
case "$INSTALL_DIR" in
  *[\"\`\$\\%]* | *'
'*) printf 'error: PAIRPANE_INSTALL_DIR contains characters a desktop entry cannot hold\n' >&2; exit 1 ;;
esac
VERSION="${PAIRPANE_VERSION:-latest}"

info() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33mwarning:\033[0m %s\n' "$*" >&2; }
fail() { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

need() { command -v "$1" >/dev/null 2>&1 || fail "'$1' is required but not installed"; }

# HTTPS only, redirects included (GitHub sends release downloads to its object storage).
download() {
  if command -v curl >/dev/null 2>&1; then
    curl --proto '=https' --proto-redir '=https' --tlsv1.2 -fsSL --retry 3 -o "$2" "$1"
  elif command -v wget >/dev/null 2>&1; then
    wget -q --https-only -O "$2" "$1"
  else
    fail "curl or wget is required"
  fi
}

case "$(uname -s)" in
  Darwin) OS="darwin" ;;
  Linux) OS="linux" ;;
  *) fail "unsupported OS: $(uname -s). On Windows use install.ps1." ;;
esac

case "$(uname -m)" in
  x86_64 | amd64) ARCH="x86_64" ;;
  arm64 | aarch64) ARCH="aarch64" ;;
  *) fail "unsupported architecture: $(uname -m)" ;;
esac

# Rosetta: prefer the native arm64 build on Apple Silicon.
if [ "$OS" = "darwin" ] && [ "$ARCH" = "x86_64" ] && [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || echo 0)" = "1" ]; then
  ARCH="aarch64"
fi

# The keys of the release's latest.json (the updater manifest), which carries each bundle's download URL.
if [ "$OS" = "linux" ]; then
  [ "$ARCH" = "x86_64" ] || fail "Linux builds are x86_64 only"
  PLATFORM="linux-x86_64-appimage"
else
  PLATFORM="darwin-$ARCH-app"
fi

if [ "$VERSION" != "latest" ] && { [ "$(printf '%s\n' "$VERSION" | wc -l)" -ne 1 ] ||
  ! printf '%s' "$VERSION" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+$'; }; then
  fail "PAIRPANE_VERSION must be a release tag like v0.2.0 (got '$VERSION')"
fi

if [ "$VERSION" = "latest" ]; then
  BASE="https://github.com/$REPO/releases/latest/download"
else
  BASE="https://github.com/$REPO/releases/download/$VERSION"
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
trap 'exit 1' INT TERM

download "$BASE/latest.json" "$TMP/latest.json" || fail "no release found at $BASE (has a release been published yet?)"
# One line, then the "url" inside this platform's object (an object holds no nested braces).
URL="$(tr -d '\n\r' <"$TMP/latest.json" | sed -n "s/.*\"$PLATFORM\"[[:space:]]*:[[:space:]]*{[^}]*\"url\"[[:space:]]*:[[:space:]]*\"\([^\"]*\)\".*/\1/p")"
TAG="$(tr -d '\n\r' <"$TMP/latest.json" | sed -n 's/^[[:space:]]*{[[:space:]]*"version"[[:space:]]*:[[:space:]]*"v\{0,1\}\([^"]*\)".*/\1/p')"
[ -n "$URL" ] || fail "this release has no $PLATFORM build"
printf '%s' "$TAG" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' || fail "latest.json announces an invalid version"
if [ "$VERSION" != "latest" ] && [ "v$TAG" != "$VERSION" ]; then
  fail "latest.json of $VERSION announces version $TAG"
fi

# Exactly this platform's artifact of that release in this repository, nothing else.
if [ "$OS" = "linux" ]; then
  ASSET="Pairpane_${TAG}_amd64.AppImage"
elif [ "$ARCH" = "aarch64" ]; then
  ASSET="Pairpane_${TAG}_aarch64.app.tar.gz"
else
  ASSET="Pairpane_${TAG}_x64.app.tar.gz"
fi
[ "$URL" = "https://github.com/$REPO/releases/download/v$TAG/$ASSET" ] || fail "latest.json points at an unexpected download"

info "Downloading $ASSET (${TAG:-$VERSION})"
download "$URL" "$TMP/$ASSET" || fail "download failed: $URL"

# Nothing is installed without a checksum to compare against.
if command -v curl >/dev/null 2>&1; then
  CODE="$(curl --proto '=https' --proto-redir '=https' --tlsv1.2 -sSL --retry 3 -o "$TMP/checksums.txt" -w '%{http_code}' "$BASE/checksums.txt" 2>/dev/null)" || CODE="000"
  case "$CODE" in
    2??) ;;
    404) fail "this release has no checksums.txt, so $ASSET can't be verified. Nothing was installed" ;;
    000) fail "could not reach $BASE/checksums.txt (network error), so $ASSET can't be verified. Nothing was installed" ;;
    429) fail "GitHub is rate limiting requests (HTTP 429 Too Many Requests) while fetching checksums.txt; try again later. Nothing was installed" ;;
    502 | 503 | 504) fail "GitHub is temporarily unavailable (HTTP $CODE) while fetching checksums.txt; try again later, see https://www.githubstatus.com. Nothing was installed" ;;
    *) fail "could not download checksums.txt (HTTP $CODE), so $ASSET can't be verified. Nothing was installed" ;;
  esac
else
  download "$BASE/checksums.txt" "$TMP/checksums.txt" 2>/dev/null || fail "could not download checksums.txt, so $ASSET can't be verified. Nothing was installed"
fi
EXPECTED="$(awk -v asset="$ASSET" '$2 == asset || $2 == "*" asset { print $1; exit }' "$TMP/checksums.txt")"
[ -n "$EXPECTED" ] || fail "checksums.txt has no entry for $ASSET"
if command -v sha256sum >/dev/null 2>&1; then
  ACTUAL="$(sha256sum "$TMP/$ASSET" | awk '{print $1}')"
else
  need shasum
  ACTUAL="$(shasum -a 256 "$TMP/$ASSET" | awk '{print $1}')"
fi
[ "$EXPECTED" = "$ACTUAL" ] || fail "checksum mismatch for $ASSET"
info "Checksum verified"

if [ "$OS" = "linux" ]; then
  mkdir -p "$INSTALL_DIR"
  chmod +x "$TMP/$ASSET"
  mv "$TMP/$ASSET" "$INSTALL_DIR/$BIN"
  info "Installed $APP_NAME ${TAG:-} to $INSTALL_DIR/$BIN"

  # AppImages mount themselves with FUSE 2, which newer distributions no longer install by default.
  if ! ldconfig -p 2>/dev/null | grep -q 'libfuse\.so\.2'; then
    warn "libfuse2 was not found. If $BIN does not start, install it (Debian/Ubuntu: sudo apt install libfuse2t64 or libfuse2)"
  fi

  if [ "${PAIRPANE_NO_SHORTCUT:-0}" != "1" ]; then
    APPS="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
    ICONS="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/256x256/apps"
    mkdir -p "$APPS" "$ICONS"
    # The app icon from the tagged sources (the AppImage keeps its own inside).
    ICON_REF="${TAG:+v$TAG}"
    download "https://raw.githubusercontent.com/$REPO/${ICON_REF:-main}/src-tauri/icons/128x128@2x.png" "$ICONS/$BIN.png" 2>/dev/null ||
      warn "couldn't download the app icon; the launcher uses a generic one"
    # StartupWMClass ties the window to this entry, so Wayland docks show the app's icon.
    cat >"$APPS/$BIN.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=$APP_NAME
Comment=Fast dual-pane file manager
Exec="$INSTALL_DIR/$BIN"
Icon=$BIN
Terminal=false
Categories=Utility;FileManager;
StartupWMClass=$BIN
EOF
    command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$APPS" >/dev/null 2>&1 || true
    # Desktops cache icons by name: refresh an existing cache so a changed icon shows without logging out. Never
    # create one, since a cache nothing else keeps up to date would hide icons other apps add later.
    THEME="$(dirname "$(dirname "$ICONS")")"
    [ -f "$THEME/icon-theme.cache" ] && command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t "$THEME" >/dev/null 2>&1 || true
    info "Added \"$APP_NAME\" to your applications menu"
  fi

  printf '\nOpen \033[1m%s\033[0m from your applications menu, or run %s. The app updates itself.\n' "$APP_NAME" "$INSTALL_DIR/$BIN"
else
  need tar
  APPS_DIR="$HOME/Applications"
  mkdir -p "$APPS_DIR" "$TMP/app"
  tar -xzf "$TMP/$ASSET" -C "$TMP/app"
  BUNDLE="$(find "$TMP/app" -maxdepth 1 -name '*.app' | head -n 1)"
  [ -n "$BUNDLE" ] || fail "$ASSET holds no .app bundle"
  rm -rf "$APPS_DIR/$APP_NAME.app"
  mv "$BUNDLE" "$APPS_DIR/$APP_NAME.app"
  # The release is not notarized: clear the download quarantine, and give Apple Silicon the ad-hoc signature it
  # needs to run unsigned code.
  xattr -dr com.apple.quarantine "$APPS_DIR/$APP_NAME.app" 2>/dev/null || true
  # Re-sign only when the existing signature does not verify: a real (Developer ID) signature must never be replaced.
  if command -v codesign >/dev/null 2>&1 && ! codesign --verify --deep "$APPS_DIR/$APP_NAME.app" >/dev/null 2>&1; then
    codesign --force --deep --sign - "$APPS_DIR/$APP_NAME.app" >/dev/null 2>&1 || true
  fi
  info "Installed $APP_NAME ${TAG:-} to $APPS_DIR/$APP_NAME.app"

  printf '\nOpen \033[1m%s\033[0m from Spotlight or Launchpad. The app updates itself.\n' "$APP_NAME"
fi
