import { describe, expect, it } from 'vitest';

import { arrive } from '../../../tests/panes';
import { arrived, asked, go, inA, shown } from '../../../tests/tabs';

describe('PaneTabs moves that follow one another', () => {
  it('a second move before the first has listed keeps the folder it passed, in the right order', async () => {
    const { backend, tabs } = await inA();
    await go(backend, tabs, '/b');
    await go(backend, tabs, '/c'); // back: a, b; shown: c

    tabs.back(); // towards /b, which has not listed yet
    tabs.back(); // already on to /a
    expect(asked(backend)).toBe('/a');
    arrived(backend);
    expect(shown(tabs)).toBe('/a');

    tabs.forward();
    expect(asked(backend)).toBe('/b');
    arrived(backend);
    tabs.forward();
    expect(asked(backend)).toBe('/c');
    arrived(backend);
    expect(tabs.canForward).toBe(false);

    tabs.back();
    arrived(backend);
    tabs.back();
    expect(asked(backend)).toBe('/a'); // and back is as it was
  });

  it('mixed moves before any has listed end where the keys lead, with the history intact', async () => {
    const { backend, tabs } = await inA();
    await go(backend, tabs, '/b');
    await go(backend, tabs, '/c');

    tabs.back(); // towards /b
    tabs.forward(); // and straight back to /c
    expect(asked(backend)).toBe('/c');
    arrived(backend);
    expect(tabs.canForward).toBe(false);

    tabs.back();
    expect(asked(backend)).toBe('/b');
    arrived(backend);
    tabs.back();
    expect(asked(backend)).toBe('/a');
  });

  it('a move that finds nothing to walk to does not count the pending one twice', async () => {
    const { backend, tabs } = await inA();
    await go(backend, tabs, '/b');

    tabs.back(); // towards /a
    tabs.back(); // nothing before /a
    tabs.back();
    arrived(backend);

    tabs.forward();
    expect(asked(backend)).toBe('/b');
    arrived(backend);
    expect(tabs.canForward).toBe(false);
  });
});

describe('PaneTabs while a tab\'s folder is still being opened', () => {
  it('a tab left before its folder has listed keeps its own folder, not the one the pane still showed', async () => {
    const { backend, tabs } = await inA();
    tabs.add();
    await go(backend, tabs, '/b'); // tabs: /a, /b
    tabs.select(0);
    arrived(backend);

    tabs.select(1); // /b is slow: the pane still shows /a
    expect(shown(tabs)).toBe('/a');
    tabs.cycle(1); // away again before it lists
    arrived(backend);

    expect(tabs.state.map((tab) => tab.path)).toEqual(['/a', '/b']);
  });

  it('the folder the session saves is the tab`s own while its folder is still being opened', async () => {
    const { backend, tabs } = await inA();
    tabs.add();
    await go(backend, tabs, '/b');
    tabs.select(0);
    arrived(backend);

    tabs.select(1);
    expect(tabs.state[1].path).toBe('/b'); // not /a, which the pane shows meanwhile

    arrived(backend);
    await go(backend, tabs, '/c');
    expect(tabs.state[1].path).toBe('/c'); // once listed, the live folder counts again
  });

  it('a new tab added while the folder of the active one is still opening gets that folder', async () => {
    const { backend, tabs } = await inA();
    tabs.add();
    await go(backend, tabs, '/b');
    tabs.select(0);
    arrived(backend);

    tabs.select(1);
    tabs.add();

    expect(tabs.state.map((tab) => tab.path)).toEqual(['/a', '/b', '/b']);
  });

  it('a tab whose folder is gone shows the home folder, as a restored one does; it does not loop', async () => {
    const { backend, tabs } = await inA();
    tabs.add();
    await go(backend, tabs, '/gone');
    tabs.select(0);
    arrived(backend);
    const refuse = () =>
      backend.last().emit({ kind: 'error', error: { code: 'notFound', message: 'No such folder.' } });

    tabs.select(1);
    refuse(); // /a is listed, so the pane would only have shown a notice
    expect(asked(backend)).toBeNull();
    arrive(backend.last(), '/home/me');

    expect(shown(tabs)).toBe('/home/me');
    expect(tabs.active).toBe(1);
    expect(tabs.state[1].path).toBe('/home/me');

    tabs.back(); // the fallback is not a step of the history: the tab goes back to where it came from
    expect(asked(backend)).toBe('/a');
    arrived(backend);
    tabs.forward();
    expect(asked(backend)).toBe('/home/me'); // and the folder that took the place of the gone one is ahead
  });

  it('home failing as well after a tab switch does not loop', async () => {
    const { backend, tabs } = await inA();
    tabs.add();
    await go(backend, tabs, '/gone');
    tabs.select(0);
    arrived(backend);
    const refuse = () =>
      backend.last().emit({ kind: 'error', error: { code: 'notFound', message: 'No such folder.' } });

    tabs.select(1);
    refuse();
    expect(asked(backend)).toBeNull();
    refuse(); // home is gone too

    expect(asked(backend)).toBe('/a'); // only the pane watching what it shows again, not another try at home
  });

  it('a tab added or left during a back move keeps that move in its own history, not the new tab', async () => {
    for (const leave of ['add', 'select'] as const) {
      const { backend, tabs } = await inA();
      if (leave === 'select') {
        tabs.add();
        arrived(backend);
        tabs.select(0);
        arrived(backend);
      }
      await go(backend, tabs, '/b'); // back: a; shown: b

      tabs.back(); // towards /a, not listed yet
      if (leave === 'add') {
        tabs.add();
      } else {
        tabs.select(1);
      }
      arrived(backend);

      const first = tabs.tabs[0];
      expect(first.path, leave).toBe('/a');
      expect(first.forward, leave).toEqual(['/b']);
      expect(first.back, leave).toEqual([]);
      expect(tabs.tabs[1].forward, leave).toEqual([]);
      expect(tabs.tabs[1].back, leave).toEqual([]);
    }
  });

  it('a back move made while a tab is still opening, and refused, still brings the tab home', async () => {
    const { backend, tabs } = await inA();
    tabs.add();
    await go(backend, tabs, '/gone'); // tab 1: back /a
    tabs.select(0);
    arrived(backend);
    const refuse = () =>
      backend.last().emit({ kind: 'error', error: { code: 'notFound', message: 'No such folder.' } });

    tabs.select(1); // /gone, not listed yet
    tabs.back(); // towards /a ... which is refused too
    refuse();

    expect(asked(backend)).toBeNull();
    arrive(backend.last(), '/home/me');
    expect(tabs.state[1].path).toBe('/home/me');
  });

  it('a folder refused after a click, not a tab switch, keeps the tab on its folder', async () => {
    const { backend, tabs } = await inA();
    tabs.add();
    arrived(backend);

    void tabs.view.listing.open('/gone');
    backend.last().emit({ kind: 'error', error: { code: 'notFound', message: 'No such folder.' } });

    expect(tabs.view.listing.notice).toBe('No such folder.');
    expect(tabs.state[1].path).toBe('/a');
  });
});
