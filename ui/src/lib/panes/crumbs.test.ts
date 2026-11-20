import { describe, expect, it } from 'vitest';

import type { Crumb } from '$lib/ipc/bindings/Crumb';
import type { Volume } from '$lib/ipc/bindings/Volume';

import { anchored, samePath, tabLabel, volumeOf } from './crumbs';

const volume = (name: string, path: string, kind: Volume['kind'] = 'drive'): Volume => ({
  name,
  label: null,
  path,
  kind,
  free: null,
});

const volumes = [
  volume('System', '/', 'system'),
  volume('Home', '/home/me', 'home'),
  volume('USB', '/media/me/USB', 'removable'),
];

/** The crumbs the backend sends for a path: the root, then one per folder. */
function crumbsOf(path: string): Crumb[] {
  const parts = path.split('/').filter((part) => part !== '');
  return [
    { name: '/', path: '/' },
    ...parts.map((name, index) => ({ name, path: `/${parts.slice(0, index + 1).join('/')}` })),
  ];
}

const names = (crumbs: Crumb[]) => crumbs.map((crumb) => crumb.name);

describe('anchored', () => {
  it('starts at the deepest volume that contains the folder, named after it', () => {
    expect(names(anchored(crumbsOf('/home/me/Projects/pairpane'), volumes))).toEqual(['Home', 'Projects', 'pairpane']);
    expect(names(anchored(crumbsOf('/media/me/USB/photos'), volumes))).toEqual(['USB', 'photos']);
    expect(names(anchored(crumbsOf('/etc'), volumes))).toEqual(['System', 'etc']);
  });

  it('keeps the paths, so every step still opens its folder', () => {
    expect(anchored(crumbsOf('/home/me/Projects'), volumes)).toEqual([
      { name: 'Home', path: '/home/me' },
      { name: 'Projects', path: '/home/me/Projects' },
    ]);
  });

  it('a volume`s own root is the whole path', () => {
    expect(names(anchored(crumbsOf('/media/me/USB'), volumes))).toEqual(['USB']);
  });

  it('without volumes the path starts at the root: System, or the drive letter on Windows', () => {
    expect(names(anchored(crumbsOf('/home/me'), []))).toEqual(['System', 'home', 'me']);

    const windows = [
      { name: 'C:\\', path: 'C:\\' },
      { name: 'Users', path: 'C:\\Users' },
    ];
    expect(names(anchored(windows, []))).toEqual(['C:', 'Users']);
    expect(names(anchored(windows, [volume('D:', 'D:\\')]))).toEqual(['C:', 'Users']);
    expect(names(anchored(windows, [volume('C:', 'C:\\')]))).toEqual(['C:', 'Users']);
  });

  it('no crumbs, no path', () => {
    expect(anchored([], volumes)).toEqual([]);
  });
});

/** The crumbs the backend sends for a Windows path. */
function windowsCrumbs(path: string): Crumb[] {
  const [drive, ...parts] = path.split('\\');
  return [
    { name: `${drive}\\`, path: `${drive}\\` },
    ...parts.map((name, index) => ({ name, path: `${drive}\\${parts.slice(0, index + 1).join('\\')}` })),
  ];
}

// sysinfo reports a folder-mounted volume with a trailing backslash; the crumbs of its folders have none.
describe('a volume mounted on a folder (trailing separator)', () => {
  const mounted = [volume('C:', 'C:\\'), volume('Data', 'C:\\Mounts\\Data\\')];

  it('anchors the path at the volume, with a path that opens', () => {
    const shown = anchored(windowsCrumbs('C:\\Mounts\\Data\\photos'), mounted);

    expect(names(shown)).toEqual(['Data', 'photos']);
    expect(shown[0].path).toBe('C:\\Mounts\\Data');
  });

  it('is the volume of its folders, and of its own root', () => {
    expect(volumeOf(windowsCrumbs('C:\\Mounts\\Data\\photos'), mounted)?.name).toBe('Data');
    expect(volumeOf(windowsCrumbs('C:\\Mounts\\Data'), mounted)?.name).toBe('Data');
    expect(volumeOf(windowsCrumbs('C:\\Users'), mounted)?.name).toBe('C:');
  });

  it('names a tab at its root after the volume', () => {
    expect(tabLabel('C:\\Mounts\\Data', mounted)).toBe('Data');
    expect(tabLabel('C:\\Mounts\\Data\\', mounted)).toBe('Data');
    expect(tabLabel('C:\\Mounts\\Other', mounted)).toBe('Other');
  });
});

describe('samePath', () => {
  it('ignores trailing separators, but keeps a root as it is', () => {
    expect(samePath('C:\\Mounts\\Data\\', 'C:\\Mounts\\Data')).toBe(true);
    expect(samePath('/media/USB/', '/media/USB')).toBe(true);
    expect(samePath('/media/USB', '/media/USB2')).toBe(false);
    expect(samePath('/', '/')).toBe(true);
    expect(samePath('/', '')).toBe(false);
  });
});

describe('volumeOf', () => {
  it('is the deepest volume the folder is in', () => {
    expect(volumeOf(crumbsOf('/home/me/Projects'), volumes)?.name).toBe('Home');
    expect(volumeOf(crumbsOf('/media/me/USB'), volumes)?.name).toBe('USB');
    expect(volumeOf(crumbsOf('/etc'), volumes)?.name).toBe('System');
  });

  it('is nothing when no volume holds the folder', () => {
    expect(volumeOf(crumbsOf('/etc'), [])).toBeUndefined();
    expect(volumeOf([], volumes)).toBeUndefined();
  });
});

describe('tabLabel', () => {
  it('is the folder`s name, with either kind of separator', () => {
    expect(tabLabel('/home/me/Projects', volumes)).toBe('Projects');
    expect(tabLabel('/home/me/Projects/', volumes)).toBe('Projects');
    expect(tabLabel('C:\\Users\\me\\Documents', [])).toBe('Documents');
  });

  it('is the volume`s name at a volume`s root', () => {
    expect(tabLabel('/media/me/USB', volumes)).toBe('USB');
    expect(tabLabel('/', volumes)).toBe('System');
    expect(tabLabel('D:\\', [volume('D:', 'D:\\')])).toBe('D:');
  });

  it('names a root without volumes, and the home folder that is not known yet', () => {
    expect(tabLabel('/', [])).toBe('System');
    expect(tabLabel('C:\\', [])).toBe('C:');
    expect(tabLabel(null, [])).toBe('Home');
  });
});
