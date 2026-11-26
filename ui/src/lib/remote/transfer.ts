import type { JobRemote } from '$lib/ipc/bindings/JobRemote';
import type { OpKind } from '$lib/ipc/bindings/OpKind';
import type { PaneTabs } from '$lib/panes/tabs.svelte';
import type { RemotePlace } from '$lib/panes/remote';

import type { OpRequest } from '$lib/ops/targets';

/** The request to copy or move the `names` in the folder of the server `from` into the folder `to` on this computer. */
export function remoteRequest(kind: Extract<OpKind, 'copy' | 'move'>, from: RemotePlace, names: string[], to: string): OpRequest {
  return {
    kind: 'remote',
    move: kind === 'move',
    from: { kind: 'remote', site: from.site, dir: from.dir },
    to: { kind: 'folder', path: to },
    names,
    shown: to,
  };
}

/** Whether `dir` is the folder `root` on a server or one below it (paths there are always `/`-separated). */
function isInside(dir: string, root: string): boolean {
  return dir === root || dir.startsWith(root.endsWith('/') ? root : `${root}/`);
}

/**
 * A job that used a folder on a server has ended (or stopped part way: a move takes items from there): the panes that
 * show that folder on that server, or one below it, list it again. Nothing is done for a job that had no server side.
 */
export function reloadRemote(panes: readonly Pick<PaneTabs, 'view'>[], remote: JobRemote | undefined) {
  if (remote === undefined) {
    return;
  }

  for (const { view } of panes) {
    const shown = view.listing.remote;
    if (shown !== undefined && shown.site === remote.site && isInside(shown.dir, remote.dir)) {
      void view.listing.reload();
    }
  }
}
