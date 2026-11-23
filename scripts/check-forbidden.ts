// Guard: reject security-sensitive patterns in authored source.
//
// Rules are data (see RULES). A line may opt out of a rule ONLY when the rule has
// `optOut: true` and the same line carries a marker such as
//   // guard-allow: html-injection
// (several ids may be comma separated). Rules with `optOut: false` can never be waived.
import { extensionOf, failWith, listFiles, matchesAny, readText } from './git-files';

interface Rule {
  id: string;
  reason: string;
  pattern: RegExp;
  /** File extensions (with dot) the rule scans. */
  extensions: readonly string[];
  /** If set, only paths matching one of these globs are scanned. */
  only?: readonly string[];
  /** Paths matching these globs are skipped for this rule. */
  skip?: readonly string[];
  /**
   * `file`: the pattern (with the `g` flag) runs over the whole file, so it can match across lines
   * (an argv array written one token per line); the violation is reported on the line it starts.
   * Default `line`: the pattern runs on each line.
   */
  scope?: 'line' | 'file';
  optOut: boolean;
}

const SOURCE = ['.rs', '.ts', '.svelte', '.js'] as const;
const UI_ONLY = ['ui/**'] as const;
const TEST_FILES = ['**/*.test.ts', 'ui/tests/**'] as const;
// A shell interpreter as a whole path segment, any case: after a path separator, `.exe` optional
// (`"/usr/bin/bash"`, `#!/bin/sh`, `\System32\cmd.exe`), or as a quoted `.exe` file name
// (`"PowerShell.exe"`, `'pwsh.exe'`). A bare quoted name such as `"bash"` is not matched: it is also
// an RPC tool name and a Markdown fence language (spawning one is caught by `shell-spawn`).
const SHELL_NAME = String.raw`(?:sh|bash|zsh|dash|fish|cmd|pwsh|powershell)`;
const SEGMENT_END = String.raw`(?![\w.-])`;
const SHELL_PROGRAM = new RegExp(
  String.raw`[/\\]${SHELL_NAME}(?:\.exe)?${SEGMENT_END}|["']${SHELL_NAME}\.exe${SEGMENT_END}`,
  'i',
);
// `Command::new` with a quoted (or raw) shell, bare or as the last segment of a path.
const SHELL_COMMAND = new RegExp(
  String.raw`Command::new\(\s*r?#*"(?:[^"]*[/\\])?${SHELL_NAME}(?:\.exe)?"`,
  'i',
);

// The one place a shell is named: the dock's command line, whose purpose is to run what the user types in their own
// shell (issue #52); the user allows it in a native dialog. Its runner falls back to the system's shell when the user's
// is not set, and its tests run one. Only these files are exempt, and only from naming a shell: it is started from the
// user's `$SHELL` / `%ComSpec%`, so `shell-spawn` holds there too. Everywhere else no marker can waive either rule.
const COMMAND_LINE = [
  'crates/pairpane_core/src/shell/mod.rs',
  'crates/pairpane_core/src/shell/tests.rs',
  'src-tauri/src/commands/shell/tests.rs',
] as const;

const RULES: readonly Rule[] = [
  {
    id: 'shell-spawn',
    reason: 'never spawn a shell; use a fixed binary with explicit argv',
    pattern: SHELL_COMMAND,
    extensions: ['.rs'],
    optOut: false,
  },
  {
    id: 'installer-shell',
    reason:
      'never name a shell interpreter in Rust source; use a fixed binary with explicit argv',
    pattern: SHELL_PROGRAM,
    extensions: ['.rs'],
    skip: COMMAND_LINE,
    optOut: false,
  },
  {
    id: 'html-injection',
    reason: '{@html} needs a sanitized component; mark the line',
    pattern: /\{@html\b/,
    extensions: ['.svelte'],
    optOut: true,
  },
  {
    id: 'inner-html',
    reason: 'assigning innerHTML/outerHTML or insertAdjacentHTML bypasses sanitization',
    pattern: /\b(?:inner|outer)HTML\b\s*(?:\+?=(?!=)|:)|\binsertAdjacentHTML\b|\bbind:innerHTML\b/,
    extensions: SOURCE,
    only: UI_ONLY,
    optOut: false,
  },
  {
    id: 'eval',
    reason: 'eval() and new Function() are forbidden (strict CSP)',
    pattern: /\beval\s*\(|\bnew\s+Function\s*\(/,
    extensions: SOURCE,
    only: UI_ONLY,
    optOut: false,
  },
  {
    id: 'insecure-updater',
    reason: 'updater must never allow an insecure transport',
    pattern: /dangerous_?[iI]nsecure_?[tT]ransport_?[pP]rotocol/,
    extensions: [...SOURCE, '.json', '.toml'],
    optOut: false,
  },
  {
    id: 'webview-network',
    reason: 'the webview must not open network connections; go through IPC (explicit exceptions need a marker)',
    pattern: /\bfetch\s*\(|\bXMLHttpRequest\b|\bWebSocket\s*\(|\bEventSource\s*\(/,
    extensions: ['.ts', '.svelte', '.js'],
    only: ['ui/src/**'],
    skip: TEST_FILES,
    optOut: true,
  },
];

// Never scanned: this guard (it names every pattern), generated code and docs.
const GLOBAL_SKIP = [
  'scripts/check-forbidden.ts',
  'ui/src/lib/ipc/bindings/**',
  'src-tauri/gen/**',
  'docs/**',
  '**/*.md',
];

const MARKER = /guard-allow:\s*([a-z0-9-]+(?:\s*,\s*[a-z0-9-]+)*)/;

function allowedIds(line: string): Set<string> {
  const match = MARKER.exec(line);
  if (!match) {
    return new Set();
  }

  return new Set(match[1].split(',').map((id) => id.trim()));
}

function appliesTo(rule: Rule, path: string): boolean {
  if (!rule.extensions.includes(extensionOf(path))) {
    return false;
  }
  if (rule.only && !matchesAny(path, rule.only)) {
    return false;
  }

  return !(rule.skip && matchesAny(path, rule.skip));
}

// A set: a line rule and a file rule with the same id may both match one line.
const violations = new Set<string>();

for (const path of listFiles()) {
  if (matchesAny(path, GLOBAL_SKIP)) {
    continue;
  }

  const rules = RULES.filter((rule) => appliesTo(rule, path));
  if (rules.length === 0) {
    continue;
  }

  const text = readText(path);
  const lines = text.split(/\r?\n/);

  const report = (rule: Rule, index: number) => {
    if (rule.optOut && allowedIds(lines[index]).has(rule.id)) {
      return;
    }

    const note = rule.optOut ? ` (opt out with "guard-allow: ${rule.id}")` : ' (no opt-out)';
    violations.add(`${path}:${index + 1} [${rule.id}] ${rule.reason}${note}`);
  };

  lines.forEach((line, index) => {
    for (const rule of rules) {
      if (rule.scope !== 'file' && rule.pattern.test(line)) {
        report(rule, index);
      }
    }
  });

  for (const rule of rules.filter((rule) => rule.scope === 'file')) {
    for (const match of text.matchAll(rule.pattern)) {
      report(rule, text.slice(0, match.index).split('\n').length - 1);
    }
  }
}

if (violations.size > 0) {
  failWith(`check-forbidden: ${violations.size} violation(s)`, [...violations]);
}

console.log(`check-forbidden: ok (${new Set(RULES.map((rule) => rule.id)).size} rules)`);
