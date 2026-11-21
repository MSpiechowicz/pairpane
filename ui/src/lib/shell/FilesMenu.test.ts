import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';

import FilesMenu from './FilesMenu.svelte';

describe('FilesMenu', () => {
  it('lists the actions, each with the key that does the same', () => {
    render(FilesMenu, { onChoose: vi.fn() });

    expect(screen.getByRole('menu', { name: 'Files' })).toBeInTheDocument();
    expect(screen.getByRole('menuitem', { name: 'New folder' })).toHaveAttribute('aria-keyshortcuts', 'F7');
    expect(screen.getByRole('menuitem', { name: 'New file' })).toHaveAttribute('aria-keyshortcuts', 'Shift+F4');
    expect(screen.getByRole('menuitem', { name: 'Rename' })).toHaveAttribute('aria-keyshortcuts', 'F2');
    expect(screen.getByRole('menuitem', { name: 'Delete' })).toHaveAttribute('aria-keyshortcuts', 'F8');
    expect(screen.getByRole('menuitem', { name: 'Create symbolic link…' })).not.toHaveAttribute('aria-keyshortcuts');
    expect(screen.getByRole('menuitem', { name: 'Create hard link…' })).toBeEnabled();
  });

  it('reports the action of the item that was clicked', async () => {
    const onChoose = vi.fn();
    render(FilesMenu, { onChoose });

    for (const name of ['New folder', 'New file', 'Rename', 'Delete', 'Create symbolic link…', 'Create hard link…']) {
      await fireEvent.click(screen.getByRole('menuitem', { name }));
    }

    expect(onChoose.mock.calls.map(([action]) => action)).toEqual([
      'newFolder',
      'newFile',
      'renameInline',
      'trash',
      'symlink',
      'hardlink',
    ]);
  });
});
