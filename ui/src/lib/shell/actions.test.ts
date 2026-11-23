import { describe, expect, it, vi } from 'vitest';

import type { PaneTabs } from '$lib/panes/tabs.svelte';

import { type ActionContext, runAction } from './actions';

function context() {
  const tabs = { add: vi.fn(), close: vi.fn(), cycle: vi.fn(), back: vi.fn(), forward: vi.fn(), view: { layout: 'detailed' } };
  const actions: ActionContext = {
    tabs: () => tabs as unknown as PaneTabs,
    active: () => 'right',
    showMenu: vi.fn(),
    files: vi.fn(),
    toggleTransfers: vi.fn(),
    view: vi.fn(),
    edit: vi.fn(),
    compareFiles: vi.fn(),
    quickView: vi.fn(),
    find: vi.fn(),
    flatView: vi.fn(),
    terminal: vi.fn(),
    palette: vi.fn(),
  };
  return { tabs, actions };
}

describe('runAction', () => {
  it('acts on the active pane: its tabs, its history, its hotlist', () => {
    const { tabs, actions } = context();

    runAction('newTab', actions);
    runAction('previousTab', actions);
    runAction('back', actions);
    runAction('hotlist', actions);
    runAction('driveMenuLeft', actions);

    expect(tabs.add).toHaveBeenCalledOnce();
    expect(tabs.cycle).toHaveBeenCalledWith(-1);
    expect(tabs.back).toHaveBeenCalledOnce();
    expect(actions.showMenu).toHaveBeenCalledWith('right', 'hotlist');
    expect(actions.showMenu).toHaveBeenCalledWith('left', 'drives');
  });

  it('the layout actions set the layout of the active pane', () => {
    const { tabs, actions } = context();

    runAction('layoutBrief', actions);
    expect(tabs.view.layout).toBe('brief');

    runAction('layoutThumbnails', actions);
    expect(tabs.view.layout).toBe('thumbnails');

    runAction('layoutDetailed', actions);
    expect(tabs.view.layout).toBe('detailed');
    expect(actions.files).not.toHaveBeenCalled();
  });

  it('hands the file actions to the files, and the rest to their own parts', () => {
    const { actions } = context();

    runAction('copy', actions);
    runAction('deletePermanently', actions);
    runAction('palette', actions);
    runAction('quickView', actions);
    runAction('flatView', actions);

    expect(actions.files).toHaveBeenNthCalledWith(1, 'copy');
    expect(actions.files).toHaveBeenNthCalledWith(2, 'deletePermanently');
    expect(actions.palette).toHaveBeenCalledOnce();
    expect(actions.quickView).toHaveBeenCalledOnce();
    expect(actions.flatView).toHaveBeenCalledOnce();
    expect(actions.files).toHaveBeenCalledTimes(2); // the flat view is not a file action
  });
});
