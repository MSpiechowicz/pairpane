import type { ConnectOutcome } from '$lib/ipc/bindings/ConnectOutcome';
import type { ConnectTarget } from '$lib/ipc/bindings/ConnectTarget';
import type { PaneId } from '$lib/ipc/bindings/PaneId';
import type { SecretKind } from '$lib/ipc/bindings/SecretKind';
import type { SecretReason } from '$lib/ipc/bindings/SecretReason';
import type { SiteId } from '$lib/ipc/bindings/SiteId';
import type { SshHost } from '$lib/ipc/bindings/SshHost';
import { IpcFailure, type Transport } from '$lib/ipc/transport';
import type { PaneTabs } from '$lib/panes/tabs.svelte';

/** The question of a server that needs a password or a key's passphrase before it lets the user in. */
export interface SecretPrompt {
  secret: SecretKind;
  /** The other kind that would let the user in too, when there is one: the dialog can be switched to it. */
  other: SecretKind | null;
  /** The last one given was not accepted. */
  wrong: boolean;
  /** Why this kind is asked after the other one was answered (the key opened but was refused); nothing else when `null`. */
  reason: SecretReason | null;
  /** Something other than a wrong secret went wrong with the last try. */
  failure: string | undefined;
  busy: boolean;
}

/**
 * Connecting to a server (SFTP) and leaving it again. The Connect dialog collects where to connect; a server that
 * wants a password (or the passphrase of a key) is asked for it in the password dialog and the connection tried
 * again with it. The secret goes with the one call, together with the kind the server asked for (a password is
 * never sent as a passphrase, nor the other way round), and is not kept here. A connection that is made is opened in
 * the active pane, at the server's home folder.
 */
export class Remote {
  /** The Connect dialog is open. */
  shown = $state(false);
  /** The hosts the user's SSH configuration names, for the suggestions of the dialog. */
  hosts = $state.raw<SshHost[]>([]);
  /** A connection is being made. */
  busy = $state(false);
  /** Why the last connection was refused, when it was not for a secret. */
  failure = $state<string | undefined>();
  /** The password (or passphrase) question on the screen, if any. */
  prompt = $state.raw<SecretPrompt | undefined>();
  /** Where the question on the screen is for: nothing but this is kept of the attempt. */
  target = $state.raw<ConnectTarget | undefined>();

  readonly #ipc: Transport;
  readonly #panes: Record<PaneId, PaneTabs>;
  readonly #active: () => PaneId;
  readonly #say: (text: string, error: boolean) => void;
  /** Counts the attempts: the answer of one that was given up (the dialog closed) is dropped. */
  #attempt = 0;
  /** The attempts whose `remote_connect` is still in flight. */
  #flight = 0;
  /** The servers an attempt of this one reached and opened in a pane: the pane may not list yet, but it is wanted. */
  readonly #claimed = new Set<SiteId>();
  /** The servers an abandoned attempt reached while another attempt was in flight: judged when none is. */
  readonly #orphans = new Set<SiteId>();

  constructor(ipc: Transport, panes: Record<PaneId, PaneTabs>, active: () => PaneId, say: (text: string, error: boolean) => void) {
    this.#ipc = ipc;
    this.#panes = panes;
    this.#active = active;
    this.#say = say;
  }

  /** A dialog has the keyboard: the shortcuts and Tab are its own then. */
  get modal(): boolean {
    return this.shown;
  }

  /** Opens the Connect dialog; the hosts the configuration names are asked for meanwhile (none when they cannot be read). */
  async show() {
    this.shown = true;
    this.failure = undefined;

    try {
      this.hosts = await this.#ipc.invoke('remote_hosts', undefined);
    } catch {
      this.hosts = [];
    }
  }

  /** Closes the dialogs; a connection still being made is given up. */
  close() {
    this.#attempt += 1;
    this.shown = false;
    this.busy = false;
    this.failure = undefined;
    this.prompt = undefined;
    this.target = undefined;
  }

  /** Connects to `target`; the server may ask for a secret first (`prompt`). */
  async connect(target: ConnectTarget) {
    if (this.busy) {
      return;
    }

    this.target = target;
    await this.#try(null);
  }

  /** The secret typed in the password dialog: the connection is tried again with it. */
  async submitSecret(secret: string) {
    if (this.prompt !== undefined && !this.prompt.busy) {
      await this.#try(secret);
    }
  }

  /**
   * The user asks for the other kind of secret (`other`) instead of the one shown: the two swap, so the way back is
   * open too. Nothing is sent, and nothing typed or said about the last try carries over.
   */
  switchSecret() {
    const prompt = this.prompt;
    if (prompt === undefined || prompt.busy || prompt.other === null) {
      return;
    }

    this.prompt = { secret: prompt.other, other: prompt.secret, wrong: false, reason: null, failure: undefined, busy: false };
  }

  /** The password dialog was left: back to the Connect dialog. */
  cancelSecret() {
    if (this.prompt?.busy !== true) {
      this.#attempt += 1;
      this.prompt = undefined;
    }
  }

  async #try(secret: string | null) {
    const target = this.target;
    if (target === undefined) {
      return;
    }

    // The kind is the one the server asked for; a secret never goes without it.
    const kind = secret === null ? null : (this.prompt?.secret ?? null);
    if (secret !== null && kind === null) {
      return;
    }

    const attempt = ++this.#attempt;
    this.#flight += 1;
    this.#working(true);

    try {
      const outcome = await this.#ipc.invoke('remote_connect', { target, secret: kind === null ? null : secret, kind });
      if (attempt === this.#attempt) {
        this.#answered(outcome);
      } else if (outcome.kind === 'connected') {
        await this.#release(outcome.site);
      }
    } catch (error) {
      if (attempt === this.#attempt) {
        this.#refused(IpcFailure.from(error).message);
      }
    } finally {
      this.#flight -= 1;
      if (this.#flight === 0) {
        await this.#sweep();
      }
    }
  }

  #working(busy: boolean) {
    this.busy = busy;
    if (this.prompt !== undefined) {
      this.prompt = { ...this.prompt, busy, failure: busy ? undefined : this.prompt.failure };
    }
  }

  #answered(outcome: ConnectOutcome) {
    if (outcome.kind === 'secretNeeded') {
      this.busy = false;
      this.prompt = { secret: outcome.secret, other: outcome.other, wrong: outcome.wrong, reason: outcome.reason, failure: undefined, busy: false };
      return;
    }

    this.#claimed.add(outcome.site);
    const place = { kind: 'remote', site: outcome.site, label: outcome.label, dir: outcome.home } as const;
    this.close();
    void this.#panes[this.#active()].view.listing.openRemote(place);
  }

  /**
   * The connection of an attempt that was given up while it was made came through: nobody is told of it, so it is
   * closed again, unless a pane shows that server or an attempt of its own claimed it. The server hands one connection
   * to every attempt at it, so while another attempt is in flight (it may be about to claim this very server) the
   * decision waits until none is (`#sweep`). A server that cannot be told so is left; it is closed with the app.
   */
  async #release(site: SiteId) {
    if (this.#wanted(site)) {
      return;
    }

    if (this.#flight > 1) {
      this.#orphans.add(site);
      return;
    }

    await this.#close(site);
  }

  /** No attempt is in flight any more: the servers that were left to wait are closed, unless one was wanted meanwhile. */
  async #sweep() {
    const sites = [...this.#orphans];
    this.#orphans.clear();

    for (const site of sites) {
      if (!this.#wanted(site)) {
        await this.#close(site);
      }
    }
  }

  async #close(site: SiteId) {
    this.#claimed.delete(site);

    try {
      await this.#ipc.invoke('remote_disconnect', { site });
    } catch {
      // Nothing is shown of it, so there is nothing to tell the user.
    }
  }

  /** The server `site` is claimed by an attempt of this one, or a pane is on it. */
  #wanted(site: SiteId): boolean {
    return this.#claimed.has(site) || this.#shows(site);
  }

  /** A pane (a tab of it too) is on the server `site`. */
  #shows(site: SiteId): boolean {
    return [this.#panes.left, this.#panes.right].some(
      (tabs) => tabs.view.listing.remote?.site === site || tabs.tabs.some((tab) => tab.remote?.site === site),
    );
  }

  #refused(message: string) {
    this.busy = false;

    if (this.prompt === undefined) {
      this.failure = message;
    } else {
      this.prompt = { ...this.prompt, busy: false, failure: message };
    }
  }

  /** Closes the connection the pane `tabs` is on; every pane and tab that was on it goes back to its last folder on disk. */
  async disconnect(tabs: PaneTabs) {
    const site = tabs.view.listing.remote?.site;
    if (site === undefined) {
      this.#say('This pane is not on a server.', true);
      return;
    }

    try {
      await this.#ipc.invoke('remote_disconnect', { site });
    } catch (error) {
      this.#say(IpcFailure.from(error).message, true);
      return;
    }

    this.#claimed.delete(site);
    this.#panes.left.dropSite(site);
    this.#panes.right.dropSite(site);
  }
}
