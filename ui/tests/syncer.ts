// Test helpers for the Synchronize dialog's state: a Syncer over a fake backend, and a scan of a few rows to act on.
import { compareBackend, facts, finished, folder, row, rows } from './compare';
import { Syncer } from '../src/lib/compare/syncer.svelte';

export const ROOTS = { left: folder('/left'), right: folder('/right') };
export const ARCHIVE = { kind: 'archive', archive: '/right/a.zip', dir: '' } as const;

/** A Syncer open on `roots`, over a fake backend that answers with `answers`. */
export function setup(answers: Parameters<typeof compareBackend>[0] = {}, roots: Parameters<Syncer['show']>[0] = ROOTS) {
  const backend = compareBackend(answers);
  const syncer = new Syncer(backend.ipc);
  syncer.show(roots, false);
  return { ...backend, syncer };
}

/** Starts a scan and lets its answer (the id) arrive. */
export async function started(s: ReturnType<typeof setup>) {
  await s.syncer.start();
  return s.scans.at(-1)!;
}

export const actions = (syncer: Syncer) => Array.from({ length: syncer.count }, (_, index) => syncer.at(index)?.action);
export const paths = (syncer: Syncer) => Array.from({ length: syncer.count }, (_, index) => syncer.at(index)?.row.path);

/** A finished scan of four rows: a new file, a folder only the right has, a newer file and a difference. */
export async function planned() {
  const s = setup();
  const scan = await started(s);
  scan.emit(
    rows(
      row('new.txt', 'leftOnly', facts({ size: 100 }), null),
      row('dir', 'rightOnly', null, facts({ kind: 'dir', size: 0, files: 3, bytes: 3000 })),
      row('over.txt', 'leftNewer', facts({ size: 5, modified: 2000 }), facts({ size: 7, modified: 1000 })),
      row('skip.txt', 'differs'),
    ),
  );
  scan.emit(finished(4));
  return s;
}
