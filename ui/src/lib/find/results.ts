import { joinPath } from '$lib/panes/archive';

/**
 * The rows of a search-results pane are named by their path below the folder searched, with `/` between the names
 * whatever the platform. These turn such a name into real paths.
 */

/** The real path of the result `rel` below the folder `root`, with the folder's own kind of separator. */
export function joinResult(root: string, rel: string): string {
  const separator = root.includes('\\') && !root.includes('/') ? '\\' : '/';
  return joinPath(root, rel.split('/').join(separator));
}

/** The folder that holds the result `rel` (a real path) and its name in it. */
export function splitResult(root: string, rel: string): { folder: string; name: string } {
  const cut = rel.lastIndexOf('/');
  if (cut < 0) {
    return { folder: root, name: rel };
  }

  return { folder: joinResult(root, rel.slice(0, cut)), name: rel.slice(cut + 1) };
}

/** Results of one folder: their names in it, and the names the pane shows them under. */
export interface ResultGroup {
  folder: string;
  names: string[];
  rels: string[];
}

/** The results `rels`, grouped by the folder that holds them, in the order their folders first appear. */
export function groupResults(root: string, rels: readonly string[]): ResultGroup[] {
  const groups = new Map<string, ResultGroup>();

  for (const rel of rels) {
    const { folder, name } = splitResult(root, rel);
    const group = groups.get(folder) ?? { folder, names: [], rels: [] };
    group.names.push(name);
    group.rels.push(rel);
    groups.set(folder, group);
  }

  return [...groups.values()];
}
