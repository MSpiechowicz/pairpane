# Performance budgets

The quality gate before networking (#24) requires Pairpane to meet its performance budgets on Linux, macOS and Windows,
and to keep tracking them. CI measures each budget on every pull request and every push to `main`. A missed budget fails
the run. The measured numbers are written to the summary of the job that measured them, so you can compare them from run
to run.

| Budget | Limit | Where it is measured | CI job |
|---|---|---|---|
| A folder of 100 000 entries shows its first batch | ≤ 200 ms (median of 5), a tripwire; the aim of ≤ 100 ms and the time over a bare `read_dir` are reported (#71) | `crates/pairpane_core/tests/budgets.rs` | Performance budgets (ubuntu-latest, macos-latest, windows-latest) |
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
- **The listing's 100 ms aim is reported, and a 200 ms tripwire is enforced.** On shared runners the first batch of the
  same folder took 54 to 114 ms from run to run, and its time over a bare `read_dir` of the folder ranged from 1.5× to
  3.8×, depending on the runner. Neither an absolute 100 ms limit nor a ratio was stable enough to gate CI. The tripwire
  catches a gross regression. Both the aim and the ratio are shown in the summary, and #71 is to make the aim hold on
  every runner so it can be enforced.
- **The listing and the bare read are each done once before they are timed,** so the system's caches are warm, as they
  are for a folder you open again.
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
