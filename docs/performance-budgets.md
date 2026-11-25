# Performance budgets

The quality gate before networking (#24) requires Pairpane to meet its performance budgets on Linux, macOS and Windows,
and to keep tracking them. CI measures each budget on every pull request and every push to `main`. A missed budget fails
the run. The measured numbers are written to the summary of the job that measured them, so you can compare them from run
to run.

| Budget | Limit | Where it is measured | CI job |
|---|---|---|---|
| A folder of 100 000 entries shows its first batch | ≤ 3× a bare `read_dir` of the same folder (median of 5); the aim of ≤ 100 ms is reported | `crates/pairpane_core/tests/budgets.rs` | Performance budgets (ubuntu-latest, macos-latest, windows-latest) |
| The same folder, listed to its end | reported only | `crates/pairpane_core/tests/budgets.rs` | Performance budgets (ubuntu-latest, macos-latest, windows-latest) |
| A 256 MiB file copied by the engine | ≥ 80 % of the speed of a plain copy on the same disk (median of 5) | `crates/pairpane_core/tests/budgets.rs` | Performance budgets (ubuntu-latest, macos-latest, windows-latest) |
| The command palette matches a query over every command and 20 000 names | ≤ 50 ms (median of 7) | `ui/src/lib/palette/budget.test.ts` | UI tests |
| The command palette opens and draws its list | ≤ 100 ms (median of 7) | `ui/src/lib/palette/budget.test.ts` | UI tests |

## How the budgets are measured

- **Medians, not single runs.** CI runners are shared machines, and one slow run must not fail a budget.
- **The copy is compared to a plain copy on the same runner.** "Near disk speed" is measured relative to a plain loop
  over a 1 MiB buffer on the same disk, run in turns with the engine's copy. A slow runner therefore slows both copies,
  and the result does not depend on how fast the runner's disk is. The engine may use the system's fast paths (reflink,
  `copy_file_range`, `clonefile`), so on some systems it beats the plain copy.
- **The listing is compared to a bare read of the same folder.** On shared runners the first batch of the same folder took
  54 to 111 ms from run to run, so an absolute limit would fail on the runner's load. A sorted listing has to read every
  name before its first batch, so a bare `read_dir` (each name and its kind) is its floor; the budget is the listing's own
  work above that, run in turns with the bare read. The 100 ms aim is still measured and shown in the summary.
- **The listing is read once before it is timed,** so the system's caches are warm, as they are for a folder you open
  again.
- **The palette budget runs in the UI tests' DOM (happy-dom),** not in the app's webview. It catches the palette's own
  work growing (matching, ranking, drawing), not the speed of a particular webview.

## Running them locally

```sh
# The listing and copy budgets need an optimised build; a plain `cargo test` skips them.
cargo test --release -p pairpane_core --test budgets -- --ignored --test-threads=1 --nocapture

# The palette budget is part of the UI tests.
bun run test ui/src/lib/palette/budget.test.ts
```

`cargo bench -p pairpane_core` (`crates/pairpane_core/benches/listing.rs`) gives a detailed criterion report on the
listing, for working on its speed.
