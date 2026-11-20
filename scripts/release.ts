// Release helper: decide the next version from Conventional Commits and apply it. Runs in the release workflow with
// the verified Bun and nothing else (no npm packages): it only reads git and edits the version files.
//
//   bun scripts/release.ts next [--root <dir>]
//       Prints JSON: {"version": "X.Y.Z" | null, "notes": "<markdown>"}. `null` when nothing since the last tag
//       releases (only chore:, docs:, ci:, test:, ... commits).
//   bun scripts/release.ts apply <version> <notes-file> [--root <dir>]
//       Sets the version in package.json, Cargo.toml and Cargo.lock and prepends the notes to CHANGELOG.md.
//
// Version rules (as semantic-release's angular defaults, plus "type!:"): a "BREAKING CHANGE:" footer or "type!:" -> major; feat -> minor;
// fix, perf or revert -> patch; anything else -> no release. Versions count from the newest vX.Y.Z tag on HEAD.
import { spawnSync } from 'node:child_process';
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

import { isSemver, rootAndArgs, setVersion } from './set-version';

export const REPO_URL = 'https://github.com/MSpiechowicz/pairpane';

export type Bump = 'major' | 'minor' | 'patch';

export interface Commit {
  hash: string;
  type: string;
  scope: string | undefined;
  subject: string;
  breaking: string | undefined;
  issues: string[];
}

const HEADER = /^(\w+)(?:\(([^)]*)\))?(!)?: (.+)$/;
/** The breaking-change note: the footer's paragraph (up to the next blank line or the end of the message). */
const BREAKING_FOOTER = /(?:^|\n)BREAKING[ -]CHANGE: ([\s\S]+?)(?=\r?\n[ \t]*\r?\n|$)/;
/** C0 control characters other than tab, newline and carriage return: never part of a real commit message. */
const CONTROL = /[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/;
const HASH = /^[0-9a-f]{40}$/;
/** "Closes #1", "Fixes #2, #3", "Refs #4" footers. */
const ISSUE_FOOTER = /^(?:close[sd]?|fix(?:e[sd])?|resolve[sd]?|refs?)\s+(#\d+(?:\s*,\s*#\d+)*)\s*$/gim;

/** Parses one commit message; `undefined` for a message that is not a Conventional Commit (or holds control
 * characters, which no real message has: they could forge entries in the notes). */
export function parseCommit(hash: string, message: string): Commit | undefined {
  if (!HASH.test(hash) || CONTROL.test(message)) {
    return undefined;
  }

  const [header = '', ...rest] = message.trim().split('\n');
  const match = HEADER.exec(header.trim());
  if (!match) {
    return undefined;
  }

  const body = rest.join('\n');
  const footer = BREAKING_FOOTER.exec(body);
  const issues = [...body.matchAll(ISSUE_FOOTER)].flatMap((footer) =>
    [...(footer[1] ?? '').matchAll(/#(\d+)/g)].map((issue) => issue[1] ?? ''),
  );

  return {
    hash,
    type: match[1] ?? '',
    scope: match[2] || undefined,
    subject: match[4] ?? '',
    breaking: match[3] ? (footer?.[1]?.trim() ?? match[4]) : footer?.[1]?.trim(),
    issues,
  };
}

/** The bump the commits call for, or `undefined` when none of them releases. */
export function bumpFor(commits: Commit[]): Bump | undefined {
  if (commits.some((commit) => commit.breaking)) {
    return 'major';
  }
  if (commits.some((commit) => commit.type === 'feat')) {
    return 'minor';
  }
  if (commits.some((commit) => ['fix', 'perf', 'revert'].includes(commit.type))) {
    return 'patch';
  }

  return undefined;
}

export function bumped(version: string, bump: Bump): string {
  const [major = 0, minor = 0, patch = 0] = version.split('.').map(Number);

  switch (bump) {
    case 'major':
      return `${major + 1}.0.0`;
    case 'minor':
      return `${major}.${minor + 1}.0`;
    case 'patch':
      return `${major}.${minor}.${patch + 1}`;
  }
}

function entry(commit: Commit): string {
  const scope = commit.scope ? `**${commit.scope}:** ` : '';
  const short = commit.hash.slice(0, 7);
  const issues = commit.issues.map((issue) => `[#${issue}](${REPO_URL}/issues/${issue})`);
  const closes = issues.length > 0 ? `, closes ${issues.join(' ')}` : '';

  return `* ${scope}${commit.subject} ([${short}](${REPO_URL}/commit/${commit.hash}))${closes}`;
}

/** Release notes in the conventional-changelog format CHANGELOG.md already uses. */
export function notesFor(previous: string, version: string, commits: Commit[], date: string): string {
  const heading = version.endsWith('.0') ? '#' : '##';
  const lines = [`${heading} [${version}](${REPO_URL}/compare/v${previous}...v${version}) (${date})`, '', ''];

  const sections: [string, Commit[]][] = [
    ['⚠ BREAKING CHANGES', commits.filter((commit) => commit.breaking)],
    ['Features', commits.filter((commit) => commit.type === 'feat')],
    ['Bug Fixes', commits.filter((commit) => commit.type === 'fix')],
    ['Performance Improvements', commits.filter((commit) => commit.type === 'perf')],
    ['Reverts', commits.filter((commit) => commit.type === 'revert')],
  ];

  for (const [title, list] of sections) {
    if (list.length === 0) {
      continue;
    }

    lines.push(`### ${title}`, '');
    for (const commit of list) {
      lines.push(title.startsWith('⚠') ? `* ${commit.breaking}` : entry(commit));
    }
    lines.push('');
  }

  return `${lines.join('\n').trimEnd()}\n`;
}

function git(root: string, args: string[]): string {
  const result = spawnSync('git', args, { cwd: root, encoding: 'utf8' });
  if (result.status !== 0) {
    throw new Error(`git ${args.join(' ')} failed: ${result.stderr.trim()}`);
  }

  return result.stdout;
}

/** The newest `vX.Y.Z` tag reachable from HEAD, as `X.Y.Z`; `0.0.0` when there is none. */
export function lastVersion(root: string): string {
  // Plain releases only: a pre-release tag (v1.0.0-rc.1) is not a release to count from.
  const tags = git(root, ['tag', '--merged', 'HEAD', '--list', 'v*.*.*'])
    .split('\n')
    .map((tag) => tag.trim().replace(/^v/, ''))
    .filter((tag) => /^\d+\.\d+\.\d+$/.test(tag));

  tags.sort((a, b) => {
    const [x, y] = [a.split('.').map(Number), b.split('.').map(Number)];

    return (x[0]! - y[0]!) || (x[1]! - y[1]!) || (x[2]! - y[2]!);
  });

  return tags.at(-1) ?? '0.0.0';
}

/** The Conventional Commits since `v<since>` (all of history when that tag does not exist), oldest first. */
export function commitsSince(root: string, since: string): Commit[] {
  const tagExists = git(root, ['tag', '--list', `v${since}`]).trim() !== '';
  const range = tagExists ? [`v${since}..HEAD`] : ['HEAD'];
  // One commit per record, separated by NUL (which a commit message cannot contain); the hash is the first line.
  const log = git(root, ['log', '-z', '--reverse', '--no-merges', '--format=%H%n%B', ...range]);

  return log
    .split('\0')
    .filter((record) => record.trim() !== '')
    .map((record) => {
      const newline = record.indexOf('\n');
      const hash = (newline === -1 ? record : record.slice(0, newline)).trim();
      const message = newline === -1 ? '' : record.slice(newline + 1);

      // Fail loudly rather than skip: a dropped feat!/fix commit would release the wrong version.
      if (CONTROL.test(message)) {
        throw new Error(`commit ${hash} has control characters in its message; fix or revert it by hand`);
      }

      return parseCommit(hash, message);
    })
    .filter((commit): commit is Commit => commit !== undefined);
}

export function next(root: string, date: string): { version: string | null; notes: string } {
  const previous = lastVersion(root);
  const commits = commitsSince(root, previous);
  const bump = bumpFor(commits);

  if (!bump) {
    return { version: null, notes: '' };
  }

  const version = bumped(previous, bump);

  return { version, notes: notesFor(previous, version, commits, date) };
}

const PACKAGE_VERSION = /^ {2}"version": "[^"]*",$/m;

/** Writes `version` into package.json (its version line only), Cargo.toml and Cargo.lock; prepends `notes`. */
export function apply(root: string, version: string, notes: string): void {
  if (!isSemver(version)) {
    throw new Error(`"${version}" is not a semver version`);
  }

  const packagePath = join(root, 'package.json');
  const packageJson = readFileSync(packagePath, 'utf8');
  if (!PACKAGE_VERSION.test(packageJson)) {
    throw new Error('package.json has no top-level "version" line');
  }
  writeFileSync(packagePath, packageJson.replace(PACKAGE_VERSION, `  "version": "${version}",`));

  setVersion(root, version);

  const changelogPath = join(root, 'CHANGELOG.md');
  const changelog = existsSync(changelogPath) ? readFileSync(changelogPath, 'utf8') : '';
  writeFileSync(changelogPath, changelog ? `${notes.trimEnd()}\n\n${changelog}` : notes);
}

function main(argv: string[]): number {
  const { root, args } = rootAndArgs(argv);
  const [command, ...rest] = args;

  try {
    if (command === 'next' && rest.length === 0) {
      const date = new Date().toISOString().slice(0, 10);
      console.log(JSON.stringify(next(root, date)));

      return 0;
    }

    if (command === 'apply' && rest.length === 2) {
      const [version = '', notesFile = ''] = rest;
      apply(root, version, readFileSync(notesFile, 'utf8'));

      return 0;
    }
  } catch (error) {
    console.error(`release: ${error instanceof Error ? error.message : String(error)}`);

    return 1;
  }

  console.error('usage: bun scripts/release.ts next | apply <version> <notes-file> [--root <dir>]');

  return 2;
}

if (import.meta.main) process.exit(main(process.argv));
