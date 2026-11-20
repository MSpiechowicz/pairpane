//! The listing budget: a folder with 100 000 entries must show its first screen in well under 100 ms.
//! `cargo bench -p pairpane_core` reports the time to the first batch and to the whole listing.

use std::fs;
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, Ordering};

use criterion::{Criterion, criterion_group, criterion_main};
use pairpane_core::listing::list;
use pairpane_core::model::ListEvent;
use tempfile::TempDir;

const ENTRIES: usize = 100_000;

fn huge_folder() -> TempDir {
    let temp = TempDir::new().expect("temp dir");
    for index in 0..ENTRIES {
        let name = format!("file-{index:06}.txt");
        if index % 10 == 0 {
            fs::create_dir(temp.path().join(name)).expect("dir");
        } else {
            fs::write(temp.path().join(name), b"x").expect("file");
        }
    }
    temp
}

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
