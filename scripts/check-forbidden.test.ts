// Run with `bun test scripts/`. Runs a copy of the guard on a scratch git repo holding one sample file,
// so the real sources are never touched. Samples that would trip the guard are assembled at run time:
// this file is itself scanned by the real guard.
import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import { cpSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';

const repoRoot = resolve(import.meta.dir, '..');

let scratch: string;

beforeEach(() => {
  scratch = mkdtempSync(join(tmpdir(), 'guard-'));
  mkdirSync(join(scratch, 'scripts'));
  for (const name of ['check-forbidden.ts', 'git-files.ts']) {
    cpSync(join(repoRoot, 'scripts', name), join(scratch, 'scripts', name));
  }
  expect(Bun.spawnSync(['git', 'init', '-q'], { cwd: scratch }).success).toBe(true);
});

afterEach(() => {
  rmSync(scratch, { recursive: true, force: true });
});

/** Runs the guard on `source` saved as `path`: the exit code and the output. */
function scan(path: string, source: string): { ok: boolean; output: string } {
  const file = join(scratch, path);
  mkdirSync(dirname(file), { recursive: true });
  writeFileSync(file, source);

  const result = Bun.spawnSync(['bun', 'scripts/check-forbidden.ts'], { cwd: scratch });

  return { ok: result.success, output: `${result.stdout}${result.stderr}` };
}

function expectViolation(path: string, source: string, id: string) {
  const { ok, output } = scan(path, source);

  expect(ok).toBe(false);
  expect(output).toContain(`[${id}]`);
}

function expectClean(path: string, source: string) {
  const { ok, output } = scan(path, source);

  expect(output).toContain('check-forbidden: ok');
  expect(ok).toBe(true);
}

// Pieces joined at run time so this file holds no literal the real guard would flag.
const MARK = ['guard', 'allow'].join('-');
const SHELL_PATH = ['', 'bin', 'sh'].join('/');
const EVAL = ['ev', 'al'].join('');
const INSECURE = ['dangerous', 'Insecure', 'Transport', 'Protocol'].join('');

describe('shell-spawn', () => {
  test('forbids Command::new with a shell', () => {
    expectViolation('src-tauri/src/run.rs', 'fn f() { Command::new("bash"); }\n', 'shell-spawn');
  });

  test('allows a fixed binary', () => {
    expectClean('src-tauri/src/run.rs', 'fn f() { Command::new("git"); }\n');
  });
});

describe('installer-shell', () => {
  test('forbids naming a shell path in Rust', () => {
    expectViolation('src-tauri/src/run.rs', `const SHELL: &str = "${SHELL_PATH}";\n`, 'installer-shell');
  });

  test('cannot be waived with a marker', () => {
    expectViolation('src-tauri/src/run.rs', `const SHELL: &str = "${SHELL_PATH}"; // ${MARK}: installer-shell\n`, 'installer-shell');
  });

  test('allows a bare tool name', () => {
    expectClean('src-tauri/src/run.rs', 'const TOOL: &str = "bash"; // a markdown fence language\n');
  });
});

describe('the command line', () => {
  test('is the one place a shell may be named and spawned', () => {
    expectClean('crates/pairpane_core/src/shell/mod.rs', `const SHELL: &str = "${SHELL_PATH}";\n`);
    expectClean('src-tauri/src/commands/shell/tests.rs', 'fn f() { Command::new("bash"); }\n');
  });

  test('does not reach past its own files', () => {
    expectViolation('crates/pairpane_core/src/shells.rs', `const SHELL: &str = "${SHELL_PATH}";\n`, 'installer-shell');
    expectViolation('src-tauri/src/commands/shell_more.rs', 'fn f() { Command::new("bash"); }\n', 'shell-spawn');
  });
});

describe('html-injection', () => {
  const html = ['{', '@html body}'].join('');

  test('forbids {@html} in Svelte', () => {
    expectViolation('ui/src/App.svelte', `<div>${html}</div>\n`, 'html-injection');
  });

  test('allows a marked line', () => {
    expectClean('ui/src/App.svelte', `<div>${html}</div> <!-- ${MARK}: html-injection -->\n`);
  });
});

describe('inner-html', () => {
  test('forbids assigning innerHTML in the UI', () => {
    expectViolation('ui/src/lib/dom.ts', 'el.innerHTML = value;\n', 'inner-html');
  });

  test('allows reading innerHTML outside the UI and textContent inside it', () => {
    expectClean('ui/src/lib/dom.ts', 'el.textContent = value;\n');
    expectClean('scripts/other.ts', 'el.innerHTML = value;\n');
  });
});

describe('eval', () => {
  test('forbids eval() in the UI', () => {
    expectViolation('ui/src/lib/run.ts', `${EVAL}("1 + 1");\n`, 'eval');
  });

  test('allows a similarly named identifier', () => {
    expectClean('ui/src/lib/run.ts', 'const retrieval = 1;\n');
  });
});

describe('insecure-updater', () => {
  test('forbids the insecure transport flag in config', () => {
    expectViolation('src-tauri/tauri.conf.json', `{ "${INSECURE}": true }\n`, 'insecure-updater');
  });

  test('allows an updater config without it', () => {
    expectClean('src-tauri/tauri.conf.json', '{ "endpoints": ["https://example.com"] }\n');
  });
});

describe('webview-network', () => {
  test('forbids fetch in the UI', () => {
    expectViolation('ui/src/lib/net.ts', 'const r = fetch("/x");\n', 'webview-network');
  });

  test('allows a marked line', () => {
    expectClean('ui/src/lib/net.ts', `const r = fetch("/x"); // ${MARK}: webview-network\n`);
  });

  test('skips test files', () => {
    expectClean('ui/src/lib/net.test.ts', 'const r = fetch("/x");\n');
  });
});
