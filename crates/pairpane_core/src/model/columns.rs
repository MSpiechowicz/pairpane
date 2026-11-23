use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The size of a picture in pixels, as its header says: for a JPEG after the turn its EXIF orientation asks for, for a HEIC
/// or AVIF after the crop and the turn of its properties.
/// Never zero on either side: a header that says so has no dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Dimensions {
    pub width: u32,
    pub height: u32,
}
