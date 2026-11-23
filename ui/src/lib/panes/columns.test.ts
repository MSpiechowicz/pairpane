import { render, screen } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';

import { DEFAULT_PREFERENCES, preferences } from '$lib/shell/preferences.svelte';

import { entry, fakeBackend } from '../../../tests/panes';
import ColumnHeader from './ColumnHeader.svelte';
import { columnTracks, shows } from './columns';
import PaneRow from './PaneRow.svelte';
import { PaneTabs } from './tabs.svelte';

const none = { ...DEFAULT_PREFERENCES, showExt: false, showSize: false, showModified: false };

describe('the grid of the columns', () => {
  it('has the icon, the name and every column the settings show', () => {
    expect(columnTracks(DEFAULT_PREFERENCES)).toEqual({
      full: '20px minmax(0, 1fr) 48px 74px 118px',
      narrow: '20px minmax(0, 1fr) 74px',
    });
  });

  it('drops a column that is hidden and keeps the others in their order', () => {
    expect(columnTracks({ ...DEFAULT_PREFERENCES, showExt: false }).full).toBe('20px minmax(0, 1fr) 74px 118px');
    expect(columnTracks({ ...DEFAULT_PREFERENCES, showSize: false }).full).toBe('20px minmax(0, 1fr) 48px 118px');
    expect(columnTracks({ ...DEFAULT_PREFERENCES, showModified: false }).full).toBe('20px minmax(0, 1fr) 48px 74px');
    expect(columnTracks(none)).toEqual({ full: '20px minmax(0, 1fr)', narrow: '20px minmax(0, 1fr)' });
  });

  it('the narrow layout has the size only when it is shown', () => {
    expect(columnTracks({ ...DEFAULT_PREFERENCES, showSize: false }).narrow).toBe('20px minmax(0, 1fr)');
  });

  it('always shows the name', () => {
    expect(shows('name', none)).toBe(true);
    expect(shows('size', none)).toBe(false);
  });
});

describe('the columns of a pane', () => {
  const headers = () => screen.getAllByRole('button').map((button) => /^\w+/.exec(button.textContent ?? '')?.[0]);
  const row = () => render(PaneRow, { id: 'r-1', index: 1, entry: entry('a.txt', 'file', { size: 5, modified: 0 }), cursor: false, selected: false, size: '5 B' });

  it('shows every column of the header and of a row by default', () => {
    render(ColumnHeader, { view: new PaneTabs(fakeBackend().ipc, 'left').view });
    row();

    expect(headers()).toEqual(['Name', 'Ext', 'Size', 'Modified']);
    for (const cell of ['.nm', '.ex', '.sz', '.dt']) {
      expect(document.querySelector(`.row ${cell}`)).not.toBeNull();
    }
  });

  it('leaves a column out of both the header and the rows when the settings hide it', () => {
    preferences.set({ showExt: false, showModified: false });
    render(ColumnHeader, { view: new PaneTabs(fakeBackend().ipc, 'left').view });
    row();

    expect(headers()).toEqual(['Name', 'Size']);
    expect(document.querySelector('.row .ex')).toBeNull();
    expect(document.querySelector('.row .dt')).toBeNull();
    expect(document.querySelector('.row .sz')).toHaveTextContent('5 B');
    expect(document.querySelector('.row .nm')).toHaveTextContent('a');
  });

  it('shows the name alone when every column is hidden', () => {
    preferences.set({ showExt: false, showSize: false, showModified: false });
    render(ColumnHeader, { view: new PaneTabs(fakeBackend().ipc, 'left').view });
    row();

    expect(headers()).toEqual(['Name']);
    expect(document.querySelectorAll('.row > span')).toHaveLength(2); // the icon and the name
  });
});
