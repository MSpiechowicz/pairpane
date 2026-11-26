import type { PaneLocation } from '$lib/ipc/bindings/PaneLocation';

/** A folder on a server a pane lists: the connection, what the path bar calls it, and the folder (a path on the server). */
export type RemotePlace = Extract<PaneLocation, { kind: 'remote' }>;

/** The place on a server a listing `location` is, or `undefined` when it is not on one. */
export function remoteOf(location: PaneLocation | undefined): RemotePlace | undefined {
  return location?.kind === 'remote' ? location : undefined;
}

/** Whether both are on the same server connection, or neither is on one. */
export function sameRemote(a: PaneLocation | undefined, b: PaneLocation | undefined): boolean {
  return remoteOf(a)?.site === remoteOf(b)?.site;
}
