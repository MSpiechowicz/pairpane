import { IpcFailure, type Transport } from '$lib/ipc/transport';
import { archiveName } from '$lib/panes/archive';
import { isWindowsPlatform } from '$lib/panes/keys';

import { type OpRequest, stemLength } from './targets';

/** What a key or a menu item asks the file actions to do. */
export type FileAction =
  | 'copy'
  | 'move'
  | 'rename'
  | 'renameInline'
  | 'newFolder'
  | 'newFile'
  | 'trash'
  | 'deletePermanently'
  | 'symlink'
  | 'hardlink'
  | 'properties'
  | 'attributes'
  | 'checksum'
  | 'verifyChecksum'
  | 'split'
  | 'combine'
  | 'extract'
  | 'pack';

/** What the OS calls the place deleted items go to. */
export const trashName = () => (isWindowsPlatform() ? 'Recycle Bin' : 'Trash');

/** `“a.pdf”` for one item, `3 items` for more. */
export function what(names: readonly string[]): string {
  return names.length === 1 ? `“${names[0]}”` : `${names.length} items`;
}

/** How a dialog looks: its title, the field's name and start, how much of it is selected, its button. */
export interface Spec {
  title: string;
  confirm: string;
  /** The accessible name of the text field. */
  label: string;
  hint?: string;
  initial: string;
  /** How much of `initial` starts selected (all of it when not given), so that typing keeps an extension. */
  stem?: number;
  /** Whether the job can be verified after it ran. */
  verify: boolean;
  /** The field only shows where the job goes: it cannot be changed. */
  fixed?: boolean;
}

/** Where an archive's folder is shown: the archive, then `▸` and the folder in it (`/` is its root). */
export function archivePlace(archive: string, dir: string): string {
  return `${archive} ▸ ${dir === '' ? '/' : dir}`;
}

export function specOf(request: OpRequest): Spec {
  switch (request.kind) {
    case 'copy':
    case 'move': {
      const verb = request.kind === 'copy' ? 'Copy' : 'Move';
      return { title: `${verb} ${what(request.names)} to`, confirm: verb, label: 'Target folder', initial: request.to, verify: true };
    }
    case 'add':
      return {
        title: `Copy ${what(request.names)} to`,
        confirm: 'Copy',
        label: 'Archive folder',
        hint: 'The archive is written again with the new items.',
        initial: archivePlace(request.archive, request.dir),
        verify: false,
        fixed: true,
      };
    case 'extract': {
      const items = request.names.length > 0 ? request.names : request.archives.map(({ archive }) => archiveName(archive));
      return { title: `Extract ${what(items)} to`, confirm: 'Extract', label: 'Target folder', initial: request.to, verify: false };
    }
    case 'combine':
      return {
        title: `Combine ${what([request.name])} to`,
        confirm: 'Combine',
        label: 'Target folder',
        hint: 'Joins .001, .002, … in order. A .crc file next to them names the new file and checks it.',
        initial: request.to,
        verify: false,
      };
    case 'rename':
      return {
        title: 'Rename',
        confirm: 'Rename',
        label: 'New name',
        initial: request.name,
        stem: stemLength(request.name, request.dir),
        verify: false,
      };
    case 'mkdir':
      return {
        title: 'New folder',
        confirm: 'Create',
        label: 'Folder name',
        hint: 'Put / between names to make folders inside folders.',
        initial: '',
        verify: false,
      };
    case 'mkfile':
      return { title: 'New file', confirm: 'Create', label: 'File name', initial: '', verify: false };
    case 'symlink':
    case 'hardlink': {
      const kind = request.kind === 'symlink' ? 'symbolic' : 'hard';
      return {
        title: `Create ${kind} link to ${what([request.name])}`,
        confirm: 'Create',
        label: 'Link name',
        initial: request.name,
        stem: stemLength(request.name, request.dir),
        verify: false,
      };
    }
  }
}

/** Something to tell the user once a dialog is done: the new file was made, but not opened. */
export interface Notice {
  text: string;
  error: boolean;
}

/** The name the owner puts its cursor on, and what to tell the user. */
export interface Done {
  name: string;
  notice?: Notice;
}

/** A new file of a program or script type is never opened; the backend says so by answering `false`. */
const NOT_OPENED = 'Created; not opened because it is a program or script';

async function makeFile(ipc: Transport, path: string, name: string): Promise<Done> {
  try {
    const opened = await ipc.invoke('ops_mkfile', { path, name });
    return opened ? { name } : { name, notice: { text: NOT_OPENED, error: false } };
  } catch (error) {
    // The file was made, only opening it failed.
    const failure = IpcFailure.from(error);
    if (failure.code !== 'process') {
      throw failure;
    }

    return { name, notice: { text: failure.message, error: true } };
  }
}

/** Asks the backend for what the dialog stands for; a refusal rejects. */
export async function perform(ipc: Transport, request: OpRequest, value: string, verify: boolean): Promise<Done> {
  switch (request.kind) {
    case 'copy':
    case 'move':
      await ipc.invoke('ops_start', { kind: request.kind, from: request.from, names: request.names, to: value, verify });
      return { name: value };
    case 'add':
      await ipc.invoke('ops_add_to_archive', { archive: request.archive, dir: request.dir, from: request.from, names: request.names });
      return { name: value };
    case 'extract':
      // One job for each archive; one that is refused stops the rest, which the dialog shows.
      for (const { archive, dir } of request.archives) {
        await ipc.invoke('ops_extract', { archive, dir, names: request.names, to: value });
      }
      return { name: value };
    case 'combine':
      await ipc.invoke('ops_task', { from: request.from, names: [request.name], task: { kind: 'combine', to: value } });
      return { name: value };
    case 'rename':
      await ipc.invoke('ops_rename', { path: request.path, from: request.name, to: value });
      return { name: value };
    case 'mkdir':
      return { name: await ipc.invoke('ops_mkdir', { path: request.path, name: value }) };
    case 'mkfile':
      return makeFile(ipc, request.path, value);
    case 'symlink':
    case 'hardlink':
      await ipc.invoke('ops_link', {
        kind: request.kind === 'symlink' ? 'symbolic' : 'hard',
        path: request.path,
        name: request.name,
        to: request.to,
        linkName: value,
      });
      return { name: value };
  }
}
