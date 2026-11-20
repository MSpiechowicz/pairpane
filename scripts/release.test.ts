import { describe, expect, test } from 'bun:test';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { apply, bumpFor, bumped, next, notesFor, parseCommit, REPO_URL } from './release';

describe('release scripts load no packages', () => {
  // The release job has no node_modules and runs Bun with --no-install: a package import would fail there, and
  // must never be added (it would be fetched and run next to a write token).
  test('scripts/release.ts and scripts/set-version.ts import only node: modules and each other', () => {
    for (const file of ['release.ts', 'set-version.ts']) {
      const source = readFileSync(join(import.meta.dir, file), 'utf8');
      // Bun's own parser: static and dynamic imports, require() and `export ... from`.
      const imports = new Bun.Transpiler({ loader: 'ts' }).scanImports(source).map((entry) => entry.path);

      expect(imports.length).toBeGreaterThan(0);
      for (const specifier of imports) {
        expect(specifier === './set-version' || specifier.startsWith('node:')).toBe(true);
      }
    }
  });
});

const HASH = 'a'.repeat(40);

describe('parseCommit', () => {
  test('reads type, scope, subject and issue footers', () => {
    const commit = parseCommit(HASH, 'fix(release): hash stdin\n\nWhy it broke.\n\nRefs #3\nCloses #4');

    expect(commit).toEqual({
      hash: HASH,
      type: 'fix',
      scope: 'release',
      subject: 'hash stdin',
      breaking: undefined,
      issues: ['3', '4'],
    });
  });

  test('marks breaking changes from "!" or the footer, keeping only the footer paragraph', () => {
    expect(parseCommit(HASH, 'feat!: new layout')?.breaking).toBe('new layout');
    expect(parseCommit(HASH, 'feat: x\n\nBREAKING CHANGE: settings moved')?.breaking).toBe('settings moved');
    expect(parseCommit(HASH, 'feat: x\n\nBREAKING CHANGE: settings moved\n\nCloses #3')?.breaking).toBe(
      'settings moved',
    );
    expect(parseCommit(HASH, 'feat: x\n\nBREAKING CHANGE: line one\nline two\n\nCloses #3')?.breaking).toBe(
      'line one\nline two',
    );
  });

  test('reads several issues from one footer', () => {
    expect(parseCommit(HASH, 'fix: x\n\nFixes #1, #2\nRefs #3')?.issues).toEqual(['1', '2', '3']);
  });

  test('ignores messages that are not Conventional Commits, control characters and bad hashes', () => {
    expect(parseCommit(HASH, 'Initial commit')).toBeUndefined();
    expect(parseCommit(HASH, 'Merge branch main')).toBeUndefined();
    expect(parseCommit(HASH, 'feat: x\n\nbody\x1edeadbeef\x1ffeat!: forged')).toBeUndefined();
    expect(parseCommit('deadbeef', 'feat: x')).toBeUndefined();
  });
});

describe('versions', () => {
  const commit = (message: string) => parseCommit(HASH, message)!;

  test('the bump follows the strongest commit', () => {
    expect(bumpFor([commit('chore: tidy'), commit('docs: readme')])).toBeUndefined();
    expect(bumpFor([commit('fix: a'), commit('perf: b')])).toBe('patch');
    expect(bumpFor([commit('revert: undo the menu change')])).toBe('patch');
    expect(bumpFor([commit('fix: a'), commit('feat: b')])).toBe('minor');
    expect(bumpFor([commit('feat: a'), commit('fix!: b')])).toBe('major');
  });

  test('bumping resets the lower parts', () => {
    expect(bumped('0.2.1', 'patch')).toBe('0.2.2');
    expect(bumped('0.2.1', 'minor')).toBe('0.3.0');
    expect(bumped('0.2.1', 'major')).toBe('1.0.0');
  });

  test('notes use the CHANGELOG format: "#" for minor/major, "##" for patches', () => {
    const notes = notesFor('0.2.0', '0.3.0', [commit('feat(ui): panes\n\nCloses #6'), commit('fix: crash')], '2026-12-01');

    expect(notes).toBe(
      [
        `# [0.3.0](${REPO_URL}/compare/v0.2.0...v0.3.0) (2026-12-01)`,
        '',
        '',
        '### Features',
        '',
        `* **ui:** panes ([aaaaaaa](${REPO_URL}/commit/${HASH})), closes [#6](${REPO_URL}/issues/6)`,
        '',
        '### Bug Fixes',
        '',
        `* crash ([aaaaaaa](${REPO_URL}/commit/${HASH}))`,
        '',
      ].join('\n'),
    );
    expect(notesFor('0.2.0', '0.2.1', [commit('fix: crash')], '2026-12-01')).toStartWith('## [0.2.1]');
  });
});

/** A scratch repository with the version files and a v0.2.0 tag. */
function scratchRepo(): string {
  const root = mkdtempSync(join(tmpdir(), 'release-test-'));
  const run = (...args: string[]) => {
    const result = spawnSync('git', ['-c', 'user.email=t@t', '-c', 'user.name=t', ...args], { cwd: root });
    if (result.status !== 0) throw new Error(String(result.stderr));
  };

  writeFileSync(join(root, 'package.json'), '{\n  "name": "pairpane",\n  "version": "0.2.0",\n  "private": true\n}\n');
  writeFileSync(join(root, 'Cargo.toml'), '[workspace.package]\nversion = "0.2.0"\n');
  writeFileSync(
    join(root, 'Cargo.lock'),
    '[[package]]\nname = "pairpane_app"\nversion = "0.2.0"\n\n[[package]]\nname = "pairpane_core"\nversion = "0.2.0"\n',
  );
  writeFileSync(join(root, 'CHANGELOG.md'), '# [0.2.0](old) (2026-11-20)\n');

  run('init', '-q');
  run('add', '-A');
  run('commit', '-q', '-m', 'feat: first');
  run('tag', 'v0.2.0');
  run('commit', '-q', '--allow-empty', '-m', 'docs: words');

  return root;
}

describe('next and apply on a repository', () => {
  test('nothing releases after only non-releasing commits', () => {
    expect(next(scratchRepo(), '2026-12-01')).toEqual({ version: null, notes: '' });
  });

  test('a commit message with control characters stops the release instead of forging or dropping entries', () => {
    const root = scratchRepo();
    const forged = 'chore: harmless\n\nbody\x1edeadbeef\x1ffeat!: forged record from body';
    spawnSync('git', ['-c', 'user.email=t@t', '-c', 'user.name=t', 'commit', '-q', '--allow-empty', '-m', forged], {
      cwd: root,
    });

    expect(() => next(root, '2026-12-01')).toThrow('control characters');
  });

  test('a pre-release tag is not counted as the last release', () => {
    const root = scratchRepo();
    spawnSync('git', ['tag', 'v9.0.0-rc.1'], { cwd: root });
    spawnSync('git', ['-c', 'user.email=t@t', '-c', 'user.name=t', 'commit', '-q', '--allow-empty', '-m', 'fix: x'], {
      cwd: root,
    });

    expect(next(root, '2026-12-01').version).toBe('0.2.1');
  });

  test('a fix since the last tag releases a patch, and apply bumps exactly the version files', () => {
    const root = scratchRepo();
    spawnSync('git', ['-c', 'user.email=t@t', '-c', 'user.name=t', 'commit', '-q', '--allow-empty', '-m', 'fix: x'], {
      cwd: root,
    });

    const plan = next(root, '2026-12-01');
    expect(plan.version).toBe('0.2.1');

    apply(root, '0.2.1', plan.notes);

    expect(readFileSync(join(root, 'package.json'), 'utf8')).toContain('  "version": "0.2.1",');
    expect(readFileSync(join(root, 'Cargo.toml'), 'utf8')).toContain('version = "0.2.1"');
    expect(readFileSync(join(root, 'Cargo.lock'), 'utf8').match(/version = "0.2.1"/g)).toHaveLength(2);
    expect(readFileSync(join(root, 'CHANGELOG.md'), 'utf8')).toStartWith('## [0.2.1]');
    expect(readFileSync(join(root, 'CHANGELOG.md'), 'utf8')).toContain('# [0.2.0](old)');
  });

  test('apply refuses a version that is not semver', () => {
    expect(() => apply(scratchRepo(), 'v1', 'notes')).toThrow('not a semver version');
  });
});
