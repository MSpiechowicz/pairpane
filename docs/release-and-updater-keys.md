# Releases and updater keys

Releases are fully automated from Conventional Commits. The only manual work is a one-time key setup (below).

## How a release happens

CI is the only go/no-go gate. The release workflow runs no tests of its own and only ever releases a commit whose
CI passed. There is no manual trigger.

1. **Commit messages decide the version.** On `main`, `fix:` releases a patch, `feat:` a minor, and a
   `BREAKING CHANGE:` footer or `feat!:` a major. Other types (`chore:`, `ci:`, `docs:`, `test:`, ...) do not
   release. The first release is 0.1.0: until a `v*` tag exists, the release job tags the root commit `v0.0.0`, so
   the first `feat:` bumps the minor version instead of semantic-release's default 1.0.0 (semantic-release pushes
   that tag along with `v0.1.0`; it has no GitHub release).
2. **CI runs** on the push to `main` (`.github/workflows/ci.yml`).
3. **The `Release` workflow** (`.github/workflows/release.yml`) starts when CI finishes. It continues only when that
   CI run **succeeded**, was a **push** to **`main`** of **this repository** (a fork's CI run never releases), and
   the commit is still on `main`. It checks out exactly the commit CI tested.
4. **Job `release`** runs `semantic-release` (config: `.releaserc.json`). If there is something to release it:
   - writes `CHANGELOG.md`;
   - bumps `package.json` (npm plugin, no publishing) and, through `scripts/set-version.ts`, the Rust workspace
     version in `Cargo.toml` and the two workspace entries (`pairpane_core`, `pairpane_app`) in `Cargo.lock`.
     `src-tauri/tauri.conf.json` reads its version from `package.json`;
   - commits them as `chore(release): X.Y.Z [skip ci]`, pushes the commit and the tag `vX.Y.Z`;
   - creates a **draft** GitHub release with the generated notes.
5. **Job `verify`** checks the release commit is exactly the CI-tested commit plus the version bump: its parent is
   the tested commit, and `package.json`, `Cargo.toml` and `Cargo.lock` equal the tested files with only the version
   replaced (`CHANGELOG.md` may change freely; nothing in the build reads it). The check is inline in the workflow
   file, which a workflow token cannot change.
6. **Job `bundle`** checks out the verified release commit, checks the tag points at it, and builds on four runners
   with a read-only token and no secrets (`bun tauri build -- --locked`):

   | Runner | Output |
   | --- | --- |
   | ubuntu-22.04 | AppImage, deb, rpm |
   | macos-latest, `aarch64-apple-darwin` | dmg, plus `.app.tar.gz` for updates |
   | macos-latest, `x86_64-apple-darwin` | dmg, plus `.app.tar.gz` for updates |
   | windows-latest | NSIS installer, MSI |

7. **Job `sign`** downloads each platform's installers into its own directory, checks each contains exactly the
   expected file names, and signs the updater artifacts with the Tauri signer (`signer sign --app-version X.Y.Z`).
   It is the only job that ever sees the signing key. It runs nothing from the repository or the release commit:
   only the Tauri CLI packages, checked against SHA-512 values pinned in the workflow file, under the runner's Node.
8. **Job `publish`** checks the exact file list, verifies every signature against the
   public key committed in `tauri.conf.json` (`minisign`) and that each is bound to its file name and this version,
   writes `latest.json` (the updater manifest, one signed entry per platform and installer type) and `checksums.txt`
   (SHA-256 of every asset, used by `install.sh`/`install.ps1`), uploads everything to the draft and only then
   publishes the release as latest.

Releases are serialized (`concurrency: release`, never cancelled). `install.sh`, `install.ps1` and the in-app
updater only ever see **published** releases, so nothing that failed CI, a bundle, the signing or the signature
check reaches users.

**Recovering a failed release.**

- A `bundle`, `sign` or `publish` job failed: re-run the failed jobs of **that same run** (Actions page, or
  `gh run rerun <run-id> --failed`); the later jobs follow.
- The `release` job itself failed **after** pushing the tag (for example the draft was not created): re-running it
  releases nothing. Delete the draft release (if any) and the tag (`git push origin :refs/tags/vX.Y.Z`), then push
  any commit to `main`; its green CI starts a fresh release of the same version.
- A job failed because of a bug in the release workflow or its scripts: re-running cannot help (the jobs use the
  workflow and scripts of the failed run's commits). Push the fix as a `fix:` commit; its green CI releases the next
  patch version, and the failed version stays an unpublished draft (delete it when you like).

## Release safeguards

- The release job fails before any tag is created when `plugins.updater.pubkey` is still the placeholder or the
  `TAURI_SIGNING_PRIVATE_KEY` secret is missing, so no release can ship that installs but never updates.
- The workflow token has no permissions by default. Only `release` (push the version commit and tag) and `publish`
  (upload, publish) can write; `bundle` and `sign` are read-only.
- **The signing key is isolated in the `sign` job**: none of the ~500 build dependencies (crates, build scripts,
  npm packages) and none of the semantic-release dependencies ever run in a job that holds it, and the job runs
  nothing a workflow token could change (no repository checkout; only pinned, hash-checked Tauri CLI packages).
  A compromised build dependency can still tamper with the installers of that one release (true of any build), but
  cannot steal the key. The key's password adds nothing inside CI (it sits next to the key); it protects the key
  file at rest.
- The release commit is checked before it is built or trusted (`verify` job): only the version may change, so a
  compromised semantic-release dependency cannot get its own code built and signed through the release commit.
- Remaining exposure, tracked separately: the `release` job runs semantic-release's npm dependencies with a token
  that can write to `main` and to releases. Published releases are protected by GitHub's immutable releases setting
  (assets and tags cannot change after publishing); the planned redesign keeps npm code away from write tokens.
- Residual risk: within one workflow run, a compromised build dependency could use the run's artifact token to
  replace another platform's artifact before `sign` downloads it; the per-platform file-name checks limit this to
  that one release. Signatures are bound to file name and version and `publish` checks both; installed apps enforce
  the version binding once the updater is configured with `requireSignedVersion` (#4).
- Bun and Node are downloaded by `.github/scripts/install-bun.sh` / `install-node.sh` and checked against pinned
  SHA-256 hashes **before** they run (the setup-* actions would execute their downloads unverified).
- `bun install --ignore-scripts` everywhere in the release: no dependency install script and no repo git hook runs.
- **No caches** in the release workflow: a cache can be written by any CI run on `main`, including CI-only dev
  dependencies, and must not reach a signed artifact.
- The bundle jobs build the release commit by SHA and fail if the tag was moved.

### Bumping Bun, Node or the Tauri CLI

`BUN_VERSION` and `NODE_VERSION` are pinned to the hashes in the install scripts. To bump Bun: download
`bun-linux-x64.zip`, `bun-darwin-aarch64.zip`, `bun-windows-x64.zip` and `SHASUMS256.txt.asc` from the Bun release,
verify the signature with Bun's release key (`F3DCC08A8572C0749B3E18888EAB4D40A7B22B59`), check the zips against
it, and put both the zip SHA-256 and the SHA-256 of the extracted `bun` / `bun.exe` into `install-bun.sh` (CI's
`BUN_VERSION` should move with it). For Node: verify the release's `SHASUMS256.txt.asc` with a key listed under
"Release keys" in the nodejs/node README and put the `linux-x64.tar.xz` hash into `install-node.sh`. When `@tauri-apps/cli` changes in `package.json`, copy the
new `integrity` values of `@tauri-apps/cli` and `@tauri-apps/cli-linux-x64-gnu` from `bun.lock` into the
"Install the Tauri signer" step of `release.yml`.

## One-time setup (you do this, locally)

Updater artifacts are signed with a Tauri signing key. The private key never goes into the repository.

1. Generate the key pair (choose a password when asked):

   ```sh
   bun tauri signer generate -w ~/.tauri/pairpane.key
   ```

   This writes the private key to `~/.tauri/pairpane.key` and the public key to `~/.tauri/pairpane.key.pub`. Back
   both up somewhere safe: **if the private key is lost, installed apps can never update again** (the public key is
   baked into every installed build). Never commit, paste or log the private key.
2. Add the repository secrets (from the repository root):

   ```sh
   gh secret set TAURI_SIGNING_PRIVATE_KEY < ~/.tauri/pairpane.key
   gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD   # prompts for the password
   ```

3. Put the **public** key (content of `~/.tauri/pairpane.key.pub`, a single base64 line) into
   `src-tauri/tauri.conf.json` under `plugins.updater.pubkey`, replacing the `NOT_CONFIGURED: ...` placeholder. The
   public key is safe to commit.

Local builds (`bun run build`) need no key: signing only happens in the release workflow.

Rotating the key means shipping one release signed by the old key that carries the new public key; after that the
old key can be retired.

## Unsigned OS installers

The macOS and Windows installers are not code-signed or notarized yet (issue #5). Updates are still signature-checked
with the updater key; only the first install shows an OS warning:

- **macOS (Gatekeeper)**: open the dmg, drag Pairpane to Applications, then right-click the app and choose **Open**,
  then **Open** again. On macOS 15 and later, if it still refuses: `System Settings > Privacy & Security` > **Open
  Anyway**.
- **Windows (SmartScreen)**: "Windows protected your PC" > **More info** > **Run anyway**.

## Known advisories

`bun run audit` (used by CI) ignores two advisories that are reachable only through `semantic-release`'s npm plugin,
which runs in the release job and is never shipped in the app:

- GHSA-3g3m-2fjr-6pw4 (glob, AbortSignal cancellation bypass) via `@semantic-release/npm` -> npm -> glob;
- GHSA-g3hv-75wg-m249 (minimatch, ReDoS through globparts) via `@semantic-release/npm` -> npm -> minimatch.

Remove the `--ignore` flags from the `audit` script in `package.json` once fixed versions are in the lockfile.
Plain `bun audit` still reports both.
