//! The listing budget: a folder with 100 000 entries must show its first screen in well under 100 ms.
//! `cargo bench -p pairpane_core` reports the time to the first batch and to the whole listing; CI checks the budget
//! itself in `tests/budgets.rs` (see `docs/performance-budgets.md`).

use std::hint::black_box;
use std::sync::atomic::{AtomicBool, Ordering};

use criterion::{Criterion, criterion_group, criterion_main};
use pairpane_core::listing::list;
use pairpane_core::model::ListEvent;

#[path = "../tests/support/huge_folder.rs"]
mod huge_folder;

use huge_folder::huge_folder;

fn listing(c: &mut Criterion) {
    let folder = huge_folder();
    let mut group = c.benchmark_group("list 100k");
    group.sample_size(10);

    // Stops after the first batch, which is what the pane renders first.
    group.bench_function("first batch", |b| {
        b.iter(|| {
            let cancel = AtomicBool::new(false);
            list(folder.path(), false, &cancel, |event| {
                if matches!(event, ListEvent::Batch { .. }) {
                    cancel.store(true, Ordering::Relaxed);
                }
                black_box(event);
            })
            .expect("listable")
        });
    });

    group.bench_function("all entries", |b| {
        b.iter(|| {
            list(folder.path(), false, &AtomicBool::new(false), |event| {
                drop(black_box(event))
            })
            .expect("listable")
        });
    });

    group.finish();
}

criterion_group!(benches, listing);
criterion_main!(benches);
