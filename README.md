# Pairpane

A fast, keyboard-first dual-pane file manager for Linux, macOS and Windows, in the spirit of Total Commander, with a
Rust core.

## Install

**Linux and macOS**

```sh
curl --proto '=https' --tlsv1.2 -fsSL https://raw.githubusercontent.com/MSpiechowicz/pairpane/main/install.sh | sh
```

Linux gets the AppImage as `~/.local/bin/pairpane` with an applications-menu entry (no root needed); macOS gets
`~/Applications/Pairpane.app`.

**Windows** (PowerShell)

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://raw.githubusercontent.com/MSpiechowicz/pairpane/main/install.ps1 | iex"
```

A per-user install with a Start menu shortcut.

Both scripts install the latest **published** release and refuse to install anything whose SHA-256 does not match the
release's `checksums.txt`. `PAIRPANE_VERSION=v0.2.0` installs a specific release.

The installers for every platform are also on the [releases page](https://github.com/MSpiechowicz/pairpane/releases).
The macOS and Windows installers are not code-signed yet: see
[unsigned installers](docs/release-and-updater-keys.md#unsigned-os-installers) for the one-time OS prompt.

## Updates

Pairpane checks for a new version when it starts and from **Check for Updates…** (the Pairpane menu on macOS, the
Help menu on Windows and Linux). It installs an update only after you confirm, and only when the download's signature
matches the key built into the app and was made for that version. On Linux the AppImage is replaced in place.

## Development

```sh
bun install
bun run dev        # app with live UI reload
bun run build      # local installers (no signing key needed)
```

Releases are automated: see [docs/release-and-updater-keys.md](docs/release-and-updater-keys.md).

## License

GPL-3.0-or-later
