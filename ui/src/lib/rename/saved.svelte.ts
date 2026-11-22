import type { RenameRule } from '$lib/ipc/bindings/RenameRule';
import type { SavedRename } from '$lib/ipc/bindings/SavedRename';
import { SavedNamed } from '$lib/saved/named.svelte';

/** The rename rules kept by name: the rule only, the items it is applied to are whichever are chosen when one is loaded. */
export class SavedRenames extends SavedNamed<SavedRename, RenameRule> {
  constructor() {
    super(
      { one: 'preset', many: 'presets' },
      (name, rule) => ({ name, rule }),
      (saved) => saved.rule,
    );
  }
}
