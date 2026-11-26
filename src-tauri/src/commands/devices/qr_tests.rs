use std::collections::BTreeSet;

use qrcodegen::{QrCode, QrCodeEcc};

use super::qr::*;

/// The dark modules a path draws, read back from `M x y h n v 1 h -n z` runs.
fn drawn(path: &str) -> BTreeSet<(i32, i32)> {
    let mut modules = BTreeSet::new();

    for run in path.split('M').filter(|run| !run.is_empty()) {
        let (start, rest) = run.split_once('h').expect("a width");
        let (x, y) = start.split_once(' ').expect("a position");
        let (x, y): (i32, i32) = (x.parse().expect("a number"), y.parse().expect("a number"));
        let (width, rest) = rest.split_once('v').expect("a height");
        let width: i32 = width.parse().expect("a number");

        assert_eq!(rest, format!("1h-{width}z"));
        modules.extend((x..x + width).map(|x| (x, y)));
    }

    modules
}

#[test]
fn the_path_draws_exactly_the_dark_modules_of_the_code() {
    let text = "5866666666666666666666666666666666666666666666666666666666666666@192.168.1.20:47620";
    let code = QrCode::encode_text(text, QrCodeEcc::Medium).expect("fits");
    let expected: BTreeSet<(i32, i32)> = (0..code.size())
        .flat_map(|y| (0..code.size()).map(move |x| (x, y)))
        .filter(|&(x, y)| code.get_module(x, y))
        .collect();

    let image = qr_image(text).expect("fits");

    assert_eq!(i32::from(image.size), code.size());
    assert_eq!(drawn(&image.path), expected);
}

#[test]
fn the_path_holds_only_what_the_page_may_draw() {
    let image = qr_image("5866666666666666666666666666666666666666666666666666666666666666").expect("fits");

    assert!(
        image.path.chars().all(|c| c.is_ascii_digit() || "Mhvz -".contains(c)),
        "{}",
        image.path
    );
}

#[test]
fn text_that_does_not_fit_has_no_code() {
    assert_eq!(qr_image(&"x".repeat(5000)), None);
}
