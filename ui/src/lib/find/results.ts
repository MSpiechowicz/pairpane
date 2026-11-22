import { joinPath } from '$lib/panes/archive';
import { isMacPlatform, isWindowsPlatform } from '$lib/panes/keys';
import type { PaneView } from '$lib/panes/view.svelte';

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

/** Windows and macOS file systems tell `Docs` from `docs` no more than a person does. */
const foldCase = (rel: string) => (isWindowsPlatform() || isMacPlatform() ? rel.toLowerCase() : rel);

/** The folders above `rel`, as names below the folder searched, from the top one down. */
function* ancestors(rel: string): Generator<string> {
  for (let cut = rel.indexOf('/'); cut >= 0; cut = rel.indexOf('/', cut + 1)) {
    yield rel.slice(0, cut);
  }
}

/**
 * The results `rels` without those that lie inside another one of them: choosing a folder and what it holds (select
 * all) is choosing the folder, and an operation must not meet the same item twice. The order is kept.
 */
export function withoutNested(rels: readonly string[]): string[] {
  const chosen = new Set(rels.map(foldCase));
  return rels.filter((rel) => ![...ancestors(rel)].some((above) => chosen.has(foldCase(above))));
}

/** `names` and the `selected` results that lie inside one of them: what a job on `names` took over. */
export function withNested(names: readonly string[], selected: readonly string[]): string[] {
  const roots = new Set(names.map(foldCase));
  const inside = selected.filter((rel) => !roots.has(foldCase(rel)) && [...ancestors(rel)].some((above) => roots.has(foldCase(above))));
  return [...names, ...inside];
}

/** Deselects the results a job took over, and the selected ones inside them (see {@link withoutNested}). */
export function deselectResults(view: PaneView, names: readonly string[]) {
  view.deselect(withNested(names, view.selectedShown().map((entry) => entry.name)));
}

/**
 * Where a result's folder is, for the backend to check on the way to it: `root` is the folder searched, `dir` the
 * folder below it (`''`: the root itself), its names joined with `/` whatever the platform.
 */
export interface Within {
  root: string;
  dir: string;
}

/** Results of one folder: their names in it, the folder below the one searched (`''`: it) and the names the pane shows them under. */
export interface ResultGroup {
  folder: string;
  dir: string;
  names: string[];
  rels: string[];
}

/**
 * The results `rels` (those inside another of them left out), grouped by the folder that holds them, in the order
 * their folders first appear.
 */
export function groupResults(root: string, rels: readonly string[]): ResultGroup[] {
  const groups = new Map<string, ResultGroup>();

  for (const rel of withoutNested(rels)) {
    const { folder, name } = splitResult(root, rel);
    const dir = rel.slice(0, Math.max(rel.lastIndexOf('/'), 0));
    const group = groups.get(folder) ?? { folder, dir, names: [], rels: [] };
    group.names.push(name);
    group.rels.push(rel);
    groups.set(folder, group);
  }

  return [...groups.values()];
}
