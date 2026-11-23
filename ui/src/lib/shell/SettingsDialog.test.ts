import { fireEvent, render, screen, within } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';

import type { Preferences as Saved } from '$lib/ipc/bindings/Preferences';

import { DEFAULT_PREFERENCES, Preferences } from './preferences.svelte';
import SettingsDialog from './SettingsDialog.svelte';

function open() {
  const preferences = new Preferences();
  const onEditSettings = vi.fn();
  const onClose = vi.fn();
  render(SettingsDialog, { preferences, onEditSettings, onClose });

  return { preferences, onEditSettings, onClose };
}

const group = (name: string) => screen.getByRole('group', { name });

/** Each checkbox: the group it is in, its label and the preference it sets. */
const CHECKBOXES: [string, string, keyof Saved][] = [
  ['Columns', 'Ext', 'showExt'],
  ['Columns', 'Size', 'showSize'],
  ['Columns', 'Modified', 'showModified'],
  ['Files', 'Show hidden files', 'showHidden'],
  ['Confirmations', 'Ask before moving to the Trash', 'confirmTrash'],
  ['Confirmations', 'Ask for the target folder when copying or moving', 'confirmTransfer'],
  ['Confirmations', 'Ask before quitting while transfers are running', 'confirmQuitWithTransfers'],
  ['Updates', 'Check for updates at start', 'checkUpdatesAtStart'],
];

describe('SettingsDialog', () => {
  it('is a dialog named Settings, with a group for each part', () => {
    open();

    expect(screen.getByRole('dialog', { name: 'Settings' })).toBeInTheDocument();
    for (const name of ['Appearance', 'Columns', 'Files', 'Confirmations', 'Editor', 'Updates']) {
      expect(group(name)).toBeInTheDocument();
    }
  });

  it('shows what is set: the defaults to start with', () => {
    open();

    expect(screen.getByRole('radio', { name: 'System' })).toBeChecked();
    for (const [name, label, key] of CHECKBOXES) {
      const box = within(group(name)).getByRole('checkbox', { name: label });
      if (DEFAULT_PREFERENCES[key]) {
        expect(box).toBeChecked();
      } else {
        expect(box).not.toBeChecked();
      }
    }
  });

  it.each(CHECKBOXES)('%s: the box "%s" sets %s, and sets it back', async (name, label, key) => {
    const { preferences } = open();
    const was = preferences.value[key];

    await fireEvent.click(within(group(name)).getByRole('checkbox', { name: label }));
    expect(preferences.value[key]).toBe(!was);

    await fireEvent.click(within(group(name)).getByRole('checkbox', { name: label }));
    expect(preferences.value[key]).toBe(was);
  });

  it('a change leaves the other preferences as they were', async () => {
    const { preferences } = open();

    await fireEvent.click(screen.getByRole('checkbox', { name: 'Show hidden files' }));

    expect(preferences.value).toEqual({ ...DEFAULT_PREFERENCES, showHidden: true });
  });

  it('chooses the theme with one of three radio buttons', async () => {
    const { preferences } = open();
    const theme = within(group('Appearance'));
    expect(theme.getAllByRole('radio').map((radio) => radio.getAttribute('name'))).toEqual(['theme', 'theme', 'theme']);

    await fireEvent.click(theme.getByRole('radio', { name: 'Dark' }));
    expect(preferences.value.theme).toBe('dark');
    expect(theme.getByRole('radio', { name: 'Dark' })).toBeChecked();
    expect(theme.getByRole('radio', { name: 'System' })).not.toBeChecked();

    await fireEvent.click(theme.getByRole('radio', { name: 'Light' }));
    expect(preferences.value.theme).toBe('light');
  });

  it('follows a preference that changes elsewhere (the Hidden button)', async () => {
    const { preferences } = open();

    preferences.set({ showHidden: true });

    await vi.waitFor(() => expect(screen.getByRole('checkbox', { name: 'Show hidden files' })).toBeChecked());
  });

  it('says where the external editor is set, and opens the settings file', async () => {
    const { onEditSettings } = open();
    const editor = within(group('Editor'));

    expect(editor.getByText(/set in the settings file/)).toBeInTheDocument();
    expect(editor.queryByRole('textbox')).not.toBeInTheDocument();
    await fireEvent.click(editor.getByRole('button', { name: 'Edit settings file…' }));

    expect(onEditSettings).toHaveBeenCalledTimes(1);
  });

  it('says that deleting for good always asks', () => {
    open();

    expect(within(group('Confirmations')).getByText('Deleting for good always asks.')).toBeInTheDocument();
  });

  it('closes with its button and with Escape', async () => {
    const { onClose } = open();

    await fireEvent.click(screen.getByRole('button', { name: 'Close' }));
    await fireEvent.keyDown(window, { key: 'Escape' });

    expect(onClose).toHaveBeenCalledTimes(2);
  });
});
