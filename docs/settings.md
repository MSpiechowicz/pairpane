# Settings

Pairpane reads a few settings from `settings.json` in its configuration folder (on Linux
`~/.config/com.mspiechowicz.pairpane/`, on macOS `~/Library/Application Support/com.mspiechowicz.pairpane/`, on Windows
`%APPDATA%\com.mspiechowicz.pairpane\`). **Settings > Edit Settings…**
creates the file with the defaults if it is not there and opens it in your system's editor for JSON files. Pairpane
never writes it after that; it is read again every time it is needed, so a change applies at once.

```json
{
  "version": 1,
  "editor": { "program": "", "args": ["{file}"] }
}
```

The rest is set in the app: **Settings > Settings…** (or "Settings…" in the command palette) has the theme, the
columns the panes show, hidden files, the confirmations that can be turned off (moving to the Trash, the target of a
copy or move, quitting while transfers run; deleting for good always asks) and whether updates are looked for at
start. Those are kept with the session in Pairpane's own state file, not in `settings.json`, which only you write.

Unknown keys are refused (a typo is reported instead of being ignored); a top-level `"$comment"` is allowed for notes.
The file may be at most 64 KiB, and it may be a link (for a dotfiles folder).

## External editor (F4)

With `editor.program` empty, F4 opens the built-in editor. With a program set, F4 starts it on the file under the
cursor:

- `program` is a bare name looked up on `PATH` (`code`, `zed`, `gedit`) or an absolute path. On macOS and Windows an
  absolute path is the safer choice: apps started from the Dock or the Start menu get a short `PATH`. On Windows give
  the `.exe` (for VS Code `C:\Users\you\AppData\Local\Programs\Microsoft VS Code\Code.exe`), not a `.cmd` wrapper.
- `args` are passed as they are, one argument each, without a shell. An argument that is exactly `{file}` is replaced
  by the file's absolute path; without one, the path is added as the last argument.
- The first time a program and its arguments are to be started in a run, Pairpane asks you to allow it. A refusal holds
  until the settings file changes.

The **Files** menu's **Edit in built-in editor** uses the built-in editor even when an external one is set. Files over
8 MiB open in the external editor if one is set, and in the read-only viewer (F3) otherwise.

Examples:

```json
{ "version": 1, "editor": { "program": "code", "args": ["--wait", "{file}"] } }
{ "version": 1, "editor": { "program": "zed", "args": ["{file}"] } }
{ "version": 1, "editor": { "program": "/usr/bin/gedit", "args": ["--new-window", "{file}"] } }
```

### A terminal editor (vim, nano, helix)

Pairpane does not look for a terminal itself: start one that runs the editor, with the file as its own argument.

```json
{ "version": 1, "editor": { "program": "kitty", "args": ["vim", "--", "{file}"] } }
{ "version": 1, "editor": { "program": "gnome-terminal", "args": ["--", "vim", "--", "{file}"] } }
{ "version": 1, "editor": { "program": "wezterm", "args": ["start", "--", "hx", "{file}"] } }
```

Prefer terminals that take the command as separate arguments (`kitty CMD…`, `gnome-terminal -- CMD…`,
`wezterm start -- CMD…`, `alacritty -e CMD…`). Some terminals and wrappers (`xterm -e` with one argument,
`x-terminal-emulator -e` on some systems, `konsole -e` in some versions) join the rest of the line into one string and
split it again, so a file name with spaces or quotes is cut up, and a crafted file name could pass options to the editor.
Put `--` before `{file}` where the editor supports it, so a name can never be read as an option.

## The command line and Open terminal here

These two are not settings, but they use what your system says:

- **The command line** (the dock's `❯` field) runs what you type in your own shell (`$SHELL`, or `/bin/sh` when it is
  not set; `%ComSpec%` on Windows, which then looks for programs on the `PATH` only, not first in the folder) in the
  folder of the active pane. Its input is closed, and its output appears above the dock: up to 1 MiB, with colours and
  other terminal codes left out. **Stop** ends the command and everything it started (what ignores the polite request
  is killed two seconds later). The first command after each start of Pairpane is shown to you in a question: if you
  allow it, later commands run without asking until Pairpane ends; if not, the command line stays off until then.
- **Open terminal here** (in the command palette, and it can be given a key) opens a terminal in that folder: on Linux
  the one `$TERMINAL` names (a program name on the `PATH` or an absolute path; a value with arguments is not used),
  otherwise the first of `x-terminal-emulator`, `gnome-terminal`, `konsole`, `xfce4-terminal`, `kitty`, `alacritty`,
  `wezterm`, `foot` and `xterm` found in the system's folders (`/usr/bin`, `/bin`, `/usr/local/bin`); Terminal on
  macOS; a new console window of your shell on Windows. The first time a terminal program is to be started after
  Pairpane starts, you are asked to allow it, with its path shown.

## Flat view (Ctrl+B)

Ctrl+B lists every file below the active pane's folder, flat, by its path below it; Ctrl+B again goes back. Hidden
files are in it when the panes show them (it is made again when that changes). Links are not followed and not listed,
and what cannot be read (no permission, a name that is not valid text) is left out; the notice says how many. Past
100 000 files the list is cut. Ctrl+B or Esc stops a walk that takes long, and what was found so far is listed.

