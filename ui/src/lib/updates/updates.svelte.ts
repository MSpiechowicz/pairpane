import type { UpdateInfo } from '$lib/ipc/bindings/UpdateInfo';
import type { UpdateProgress } from '$lib/ipc/bindings/UpdateProgress';
import { IpcFailure, type Transport } from '$lib/ipc/transport';

/** Where the updater is; the update card renders one view per kind. */
export type UpdateState =
  | { kind: 'idle' }
  | { kind: 'checking' }
  | { kind: 'upToDate'; version: string | undefined }
  | { kind: 'available'; info: UpdateInfo }
  | { kind: 'installing'; info: UpdateInfo; progress: UpdateProgress | undefined }
  | { kind: 'ready'; info: UpdateInfo }
  | { kind: 'failed'; message: string; info: UpdateInfo | undefined };

/** What the user is told for each error code; the backend message stays in the log. */
function describe(error: unknown, action: 'check' | 'install'): string {
  const code = IpcFailure.from(error).code;

  switch (code) {
    case 'forbidden':
      return 'The update failed a security check and was not installed.';
    case 'io':
    case 'timeout':
      return 'Could not reach the update server. Check your connection and try again.';
    case 'unsupported':
      return 'Updates are not available for this build.';
    case 'process':
      return 'The installer could not finish. Try again, or install the new version from the releases page.';
    case 'protocol':
      return 'The update information was not valid. Try again later.';
    default:
      return action === 'check' ? 'Checking for updates failed.' : 'Installing the update failed.';
  }
}

/**
 * The in-app updater as the UI sees it. Checks run in the backend (the webview never touches the network); an
 * update is installed only after the user confirms it, and a check at startup stays silent unless it finds one.
 */
export class Updates {
  state = $state<UpdateState>({ kind: 'idle' });

  readonly #ipc: Transport;
  readonly #currentVersion: () => string | undefined;
  readonly #mayRelaunch: () => Promise<boolean>;
  readonly #atStart: () => boolean;
  /** Bumped by every check: only the latest check may change the state. */
  #checks = 0;
  /** Installed in this session but not running until a restart: never offered again. */
  #installed: string | undefined;

  /**
   * `mayRelaunch` is asked before anything that ends the app (an install, where the installer takes over and exits
   * the app, and a restart: neither can be stopped once begun), so only this can keep them from losing unsaved
   * changes. It resolves at once when there are none. `atStart` says whether the check at startup is wanted (a setting).
   */
  constructor(
    ipc: Transport,
    currentVersion: () => string | undefined = () => undefined,
    mayRelaunch: () => Promise<boolean> = () => Promise.resolve(true),
    atStart: () => boolean = () => true,
  ) {
    this.#ipc = ipc;
    this.#currentVersion = currentVersion;
    this.#mayRelaunch = mayRelaunch;
    this.#atStart = atStart;
  }

  /** The check at startup: silent unless a newer release exists, and not made at all when the settings say not to. */
  async checkAtStart(): Promise<void> {
    if (this.#atStart()) {
      await this.check(false);
    }
  }

  get busy(): boolean {
    return this.state.kind === 'checking' || this.state.kind === 'installing';
  }

  /** Asks the backend for a newer release. `manual` (the menu) also reports "up to date" and failures. */
  async check(manual: boolean): Promise<void> {
    if (this.busy || this.state.kind === 'ready') {
      return;
    }

    const check = ++this.#checks;
    const before = this.state;
    if (manual) {
      this.state = { kind: 'checking' };
    }

    // A later check, or a silent check overtaken by anything the user did meanwhile, must not touch the state.
    const stale = () => check !== this.#checks || (!manual && this.state !== before);

    try {
      const found = await this.#ipc.invoke('update_check', undefined);
      if (stale()) {
        return;
      }

      const info = found && found.version !== this.#installed ? found : null;
      if (info) {
        this.state = { kind: 'available', info };
      } else if (manual) {
        this.state = { kind: 'upToDate', version: this.#installed ?? this.#currentVersion() };
      }
    } catch (error) {
      // A startup check never interrupts: offline or a server hiccup is not the user's problem.
      if (manual && !stale()) {
        this.state = { kind: 'failed', message: describe(error, 'check'), info: undefined };
      }
    }
  }

  /** Downloads, verifies and installs the announced update. */
  async install(): Promise<void> {
    const current = this.state;
    const info = current.kind === 'available' || current.kind === 'failed' ? current.info : undefined;
    if (!info) {
      return;
    }

    // On Windows the installer exits the app: the editor's unsaved changes are asked about first (elsewhere
    // that costs nothing unless there are some). No is the end of it; the update stays on offer.
    if (!(await this.#mayRelaunch())) {
      return;
    }

    this.state = { kind: 'installing', info, progress: undefined };

    try {
      await this.#ipc.invoke('update_install', {
        version: info.version,
        onProgress: (progress) => {
          if (this.state.kind === 'installing') {
            this.state = { kind: 'installing', info, progress };
          }
        },
      });

      // On Windows the installer has taken over and the app is exiting; elsewhere the new version runs after a restart.
      this.#installed = info.version;
      this.state = { kind: 'ready', info };
    } catch (error) {
      this.state = { kind: 'failed', message: describe(error, 'install'), info };
    }
  }

  /** Restarts into the installed version. */
  async relaunch(): Promise<void> {
    if (!(await this.#mayRelaunch())) {
      return;
    }

    await this.#ipc.invoke('app_relaunch', undefined);
  }

  /** Hides the card until the next check. */
  dismiss(): void {
    if (!this.busy) {
      this.state = { kind: 'idle' };
    }
  }
}
