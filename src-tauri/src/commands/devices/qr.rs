//! The QR code of this device's address, as one SVG path the page draws itself (no image is made here).

use std::fmt::Write;

use pairpane_core::model::QrImage;
use qrcodegen::{QrCode, QrCodeEcc};

/// `text` as a QR code of medium error correction; `None` when it does not fit into one.
pub fn qr_image(text: &str) -> Option<QrImage> {
    let code = QrCode::encode_text(text, QrCodeEcc::Medium).ok()?;
    let size = code.size();

    Some(QrImage {
        size: u16::try_from(size).ok()?,
        path: svg_path(size, |x, y| code.get_module(x, y)),
    })
}

/// The dark modules of a `size` by `size` grid as `M x y h n v 1 h -n z`, one run of dark modules in a row at a time.
fn svg_path(size: i32, dark: impl Fn(i32, i32) -> bool) -> String {
    let mut path = String::new();

    for y in 0..size {
        let mut x = 0;

        while x < size {
            if !dark(x, y) {
                x += 1;

                continue;
            }

            let start = x;
            while x < size && dark(x, y) {
                x += 1;
            }

            let run = x - start;
            // Writing to a string cannot fail.
            let _ = write!(path, "M{start} {y}h{run}v1h-{run}z");
        }
    }

    path
}
