//! HEIC through the system's libheif on Linux and the BSDs, opened at run time with `dlopen` (the app neither links nor
//! ships it, so it starts and works without it).
//!
//! The library is opened only in the helper process that decodes a picture (see `process.rs`), never in the app's own
//! process, so its initialisation code, its plugins and a crash of any of them on a hostile file stay in a process
//! that lives for one picture. It is the system's own libheif (found by the loader's usual search for `libheif.so.1`),
//! the same code the user's other programs load. It is opened once per process and never unloaded, because the
//! function pointers taken from it must stay valid and a plugin libheif loaded later (libde265, ffmpeg) keeps state
//! in it. A symbol that is missing makes the whole library count as absent, except the optional ones (`heif_init`,
//! which libheif has only since 1.13, and the two ways to bound a picture, see [`Api::bound`]); a `heif_init` that
//! fails does the same.
//!
//! Every call is made through a small guard that releases what libheif handed out (context, image handle, image) on
//! every path, and the input bytes outlive the context that reads them without copying. What libheif reports about the
//! picture is checked before it is trusted. libheif's own message text goes to the debug log only, never to the page.
#![allow(
    unsafe_code,
    reason = "dlopen and calls into the system's libheif; see `Api::load` and `Api::decode`"
)]
#![deny(clippy::undocumented_unsafe_blocks)]

use std::ffi::{CStr, c_char, c_int, c_void};
use std::io;
use std::ptr;
use std::sync::OnceLock;

use libloading::Library;

use super::{HEADER, MAX_IMAGE_PIXELS, MAX_IMAGE_SIDE, check_size, copy_rows, map_heif, output, undecodable};

/// The library, by the name the loader finds (the soname of libheif 1.x).
const LIBRARY: &str = "libheif.so.1";

/// `heif_colorspace_RGB`.
const COLORSPACE_RGB: c_int = 1;

/// `heif_chroma_interleaved_RGBA`: 8 bits per channel, R, G, B, A in one plane.
const CHROMA_INTERLEAVED_RGBA: c_int = 11;

/// `heif_channel_interleaved`.
const CHANNEL_INTERLEAVED: c_int = 10;

/// The start of `heif_security_limits` (libheif 1.19 and later): the fields of `version` 1 and, after them, the first
/// of `version` 2 (libheif 1.20), `max_total_memory`, which is read and written only when the struct says it has
/// it. Later versions only add fields after these. The struct is always the library's own, read and written through a
/// pointer that `heif_context_get_security_limits` gives, so the fields after these never matter here.
#[repr(C)]
struct Limits {
    version: u8,
    max_image_size_pixels: u64,
    max_number_of_tiles: u64,
    max_bayer_pattern_pixels: u32,
    max_items: u32,
    max_color_profile_size: u32,
    max_memory_block_size: u64,
    max_components: u32,
    max_iloc_extents_per_item: u32,
    max_size_entity_group: u32,
    max_children_per_box: u32,
    // Version 2.
    max_total_memory: u64,
}

/// The most bytes libheif may allocate as one block (a plane of the biggest picture in 16 bits per channel is 512 MiB).
const MAX_BLOCK: u64 = 1024 * 1024 * 1024;

/// The most bytes libheif may allocate all together (libheif 1.20 and later), under the address space the helper is
/// given (`sandbox.rs`) so that libheif's own refusal comes first, as an error and not as a failed allocation.
const MAX_TOTAL: u64 = 2 * 1024 * 1024 * 1024;

/// The most threads one decoding uses.
const MAX_THREADS: c_int = 4;

/// The side of the square of [`MAX_IMAGE_PIXELS`], for libheif before 1.19, which can limit only the side of a picture.
/// A picture that is wider or taller than this is refused there even when its pixels are under the cap (16384 by
/// 4096); the size check of [`Api::decode`] is the exact one.
const SQUARE_SIDE: c_int = {
    let side = MAX_IMAGE_PIXELS.isqrt();
    assert!(side * side <= MAX_IMAGE_PIXELS && side <= MAX_IMAGE_SIDE as u64);

    side as c_int
};

/// `heif_error` by value: `code` 0 (`heif_error_Ok`) is success. `message` belongs to libheif and is valid until the
/// next call on the object that returned it.
#[repr(C)]
struct HeifError {
    code: c_int,
    subcode: c_int,
    message: *const c_char,
}

type Alloc = unsafe extern "C" fn() -> *mut c_void;
type Free = unsafe extern "C" fn(*mut c_void);
type ReadMemory = unsafe extern "C" fn(*mut c_void, *const c_void, usize, *const c_void) -> HeifError;
type PrimaryHandle = unsafe extern "C" fn(*mut c_void, *mut *mut c_void) -> HeifError;
type Dimension = unsafe extern "C" fn(*const c_void) -> c_int;
type Release = unsafe extern "C" fn(*const c_void);
type Decode = unsafe extern "C" fn(*const c_void, *mut *mut c_void, c_int, c_int, *const c_void) -> HeifError;
type ChannelDimension = unsafe extern "C" fn(*const c_void, c_int) -> c_int;
type Plane = unsafe extern "C" fn(*const c_void, c_int, *mut c_int) -> *const u8;
type GetLimits = unsafe extern "C" fn(*const c_void) -> *mut Limits;
type SetImageSideLimit = unsafe extern "C" fn(*mut c_void, c_int);
type SetThreads = unsafe extern "C" fn(*mut c_void, c_int);

/// The functions of libheif used here. `library` is kept so they stay valid; the `Api` of [`api`] is static, so it is
/// never dropped and the library is never unloaded.
pub(in crate::view) struct Api {
    library: Library,
    context_alloc: Alloc,
    context_free: Free,
    read_memory: ReadMemory,
    primary_handle: PrimaryHandle,
    handle_width: Dimension,
    handle_height: Dimension,
    handle_release: Release,
    decode_image: Decode,
    image_width: ChannelDimension,
    image_height: ChannelDimension,
    plane: Plane,
    image_release: Release,
    /// Since libheif 1.19: the limits of a context, to lower in place.
    get_limits: Option<GetLimits>,
    /// Before that (deprecated since): one limit, for the side of a square picture.
    set_side_limit: Option<SetImageSideLimit>,
    /// Since libheif 1.9: the most threads one decoding uses.
    set_threads: Option<SetThreads>,
}

/// The system's libheif, opened on first use; `None` when it is not there (or lacks a function).
static LIBRARY_API: OnceLock<Option<Api>> = OnceLock::new();

/// The system's libheif, or `None` when this system has none.
pub(super) fn api() -> Option<&'static Api> {
    LIBRARY_API.get_or_init(|| Api::load(LIBRARY)).as_ref()
}

impl Api {
    /// Opens the library `name` and takes the functions from it; `None` when it cannot be opened or a function is
    /// missing. Initialises libheif with its defaults when it has `heif_init` (the plugins are loaded there).
    pub(in crate::view) fn load(name: &str) -> Option<Self> {
        // SAFETY: opening the system's libheif runs its initialisation code in this process (see the module); it is
        // trusted system code, and a failure is only reported as an error.
        let library = match unsafe { Library::new(name) } {
            Ok(library) => library,
            Err(error) => {
                tracing::debug!(%error, name, "libheif cannot be opened");

                return None;
            }
        };

        // SAFETY: each type below is the C signature of the libheif function of that name (libheif 1.12 and later;
        // see `heif_*.h`), and `symbol` copies it out as a plain function pointer that stays valid because `library`
        // is kept in the `Api` and never unloaded.
        let api = unsafe {
            Self {
                context_alloc: symbol::<Alloc>(&library, b"heif_context_alloc\0")?,
                context_free: symbol::<Free>(&library, b"heif_context_free\0")?,
                read_memory: symbol::<ReadMemory>(&library, b"heif_context_read_from_memory_without_copy\0")?,
                primary_handle: symbol::<PrimaryHandle>(&library, b"heif_context_get_primary_image_handle\0")?,
                handle_width: symbol::<Dimension>(&library, b"heif_image_handle_get_width\0")?,
                handle_height: symbol::<Dimension>(&library, b"heif_image_handle_get_height\0")?,
                handle_release: symbol::<Release>(&library, b"heif_image_handle_release\0")?,
                decode_image: symbol::<Decode>(&library, b"heif_decode_image\0")?,
                image_width: symbol::<ChannelDimension>(&library, b"heif_image_get_width\0")?,
                image_height: symbol::<ChannelDimension>(&library, b"heif_image_get_height\0")?,
                plane: symbol::<Plane>(&library, b"heif_image_get_plane_readonly\0")?,
                image_release: symbol::<Release>(&library, b"heif_image_release\0")?,
                get_limits: symbol::<GetLimits>(&library, b"heif_context_get_security_limits\0"),
                set_side_limit: symbol::<SetImageSideLimit>(&library, b"heif_context_set_maximum_image_size_limit\0"),
                set_threads: symbol::<SetThreads>(&library, b"heif_context_set_max_decoding_threads\0"),
                library,
            }
        };

        if !api.init() {
            return None;
        }

        Some(api)
    }

    /// Calls `heif_init(NULL)` once if this libheif has it; `false` when it has it and it fails (the library then counts
    /// as absent). It is never undone: the library stays loaded for the life of the process.
    ///
    /// A libheif before 1.13 has no `heif_init` and loads its plugins on the first `heif_context_alloc` instead. That
    /// must happen here, before the helper gives up the file system (`sandbox.rs`), or it would fail there and the
    /// system would be taken for one without a decoder: so one context is made and freed at once. (There is no libheif
    /// of that age to run this against in the tests; the call is the one `decode` makes anyway.)
    fn init(&self) -> bool {
        type Init = unsafe extern "C" fn(*mut c_void) -> HeifError;

        // SAFETY: `Init` is the C signature of `heif_init` (`heif_error heif_init(heif_init_params*)`), looked up
        // optionally; the symbol is copied out as a function pointer valid as long as the library, which is kept.
        let Some(init) = (unsafe { symbol::<Init>(&self.library, b"heif_init\0") }) else {
            // The plugins of this old libheif load with its first context: make one now and free it at once.
            return Context::new(self).is_ok();
        };

        // SAFETY: NULL asks for the default parameters, which `heif_init` documents; the result is read at once.
        let result = unsafe { init(ptr::null_mut()) };
        if let Err(error) = check(&result) {
            tracing::debug!(%error, "heif_init failed");

            return false;
        }

        true
    }

    /// Lowers what libheif accepts for a picture, before the file is read: with the security limits of libheif 1.19 and
    /// later the pixels and the biggest block of memory (and, from 1.20, all the memory; never raised above what the
    /// library already has, and a limit the library has disabled with 0 is set), with older libheif the side of a square
    /// of the pixel cap ([`SQUARE_SIDE`]). A libheif with neither is left to the size check of [`Api::decode`] (the
    /// picture is checked before it is decoded).
    fn bound(&self, context: &Context<'_>) {
        // Each thread of the decoder reserves an arena of address space (see `sandbox.rs`): on a machine with many cores
        // the default would use up what the helper is given.
        if let Some(set) = self.set_threads {
            // SAFETY: `context.raw` is a live context; the number is a small positive `c_int`.
            unsafe { set(context.raw, MAX_THREADS) }
        }

        if let Some(get) = self.get_limits {
            // SAFETY: `context.raw` is a live context; the result is NULL or a struct of the context that stays valid
            // until the context is freed and that the library documents as changeable in place.
            let limits = unsafe { get(context.raw) };

            // SAFETY: `limits` is checked for NULL first, and every struct with a `version` of 1 or more starts with
            // the version 1 fields of `Limits` (the header of libheif 1.19 and later), and one with a `version` of 2
            // or more has `max_total_memory` after them (libheif 1.20); each field is read and written once through
            // a raw pointer, so no reference to the longer struct exists.
            unsafe {
                if !limits.is_null() && (*limits).version >= 1 {
                    let pixels = &raw mut (*limits).max_image_size_pixels;
                    pixels.write(lowered(pixels.read(), MAX_IMAGE_PIXELS));
                    let block = &raw mut (*limits).max_memory_block_size;
                    block.write(lowered(block.read(), MAX_BLOCK));

                    if (*limits).version >= 2 {
                        let total = &raw mut (*limits).max_total_memory;
                        total.write(lowered(total.read(), MAX_TOTAL));
                    }

                    return;
                }
            }
        }

        if let Some(set) = self.set_side_limit {
            // SAFETY: `context.raw` is a live context; the side is a small positive number that fits `c_int`.
            unsafe { set(context.raw, SQUARE_SIDE) }
        }
    }

    /// The picture of the HEIC file `bytes` as the header and the RGBA pixels (see the parent module).
    pub(super) fn decode(&self, bytes: &[u8]) -> io::Result<Vec<u8>> {
        let context = Context::new(self)?;
        self.bound(&context);

        // SAFETY: `context.raw` is a live context; `bytes` is valid for `bytes.len()` bytes and outlives the context
        // (it is a parameter of this function, and `context` is dropped first), which is what reading "without copy"
        // needs. The options are NULL, which libheif documents as the only value.
        let read = unsafe { (self.read_memory)(context.raw, bytes.as_ptr().cast(), bytes.len(), ptr::null()) };
        check(&read)?;

        let mut raw_handle = ptr::null_mut();
        // SAFETY: `context.raw` is live and `raw_handle` is a valid place for the handle that is written on success.
        let primary = unsafe { (self.primary_handle)(context.raw, &raw mut raw_handle) };
        check(&primary)?;
        let handle = Handle {
            api: self,
            raw: raw_handle,
        };
        if handle.raw.is_null() {
            return Err(undecodable());
        }

        // The size the file declares is checked before anything is decoded to it.
        // SAFETY: `handle.raw` is a live image handle.
        let (declared_width, declared_height) =
            unsafe { ((self.handle_width)(handle.raw), (self.handle_height)(handle.raw)) };
        check_size(i64::from(declared_width), i64::from(declared_height))?;

        let mut raw_image = ptr::null_mut();
        // SAFETY: `handle.raw` is live, `raw_image` is a valid place for the image written on success, and NULL options
        // mean the defaults, which apply the rotation and mirroring the file asks for.
        let decoded = unsafe {
            (self.decode_image)(
                handle.raw,
                &raw mut raw_image,
                COLORSPACE_RGB,
                CHROMA_INTERLEAVED_RGBA,
                ptr::null(),
            )
        };
        check(&decoded)?;
        let image = Image {
            api: self,
            raw: raw_image,
        };
        if image.raw.is_null() {
            return Err(undecodable());
        }

        image.pixels()
    }
}

/// The limit `current` lowered to `cap`: a limit of 0 is disabled in libheif, and a higher one is too loose.
fn lowered(current: u64, cap: u64) -> u64 {
    if current == 0 || current > cap { cap } else { current }
}

/// The function `name` (NUL-terminated) of `library` as a plain function pointer, or `None` when it has none.
///
/// # Safety
///
/// `T` must be the exact type of the function the name stands for, and a function pointer.
unsafe fn symbol<T: Copy>(library: &Library, name: &[u8]) -> Option<T> {
    // SAFETY: the caller promises that `T` is the function's type; the pointer is copied out and the borrow ends here.
    unsafe { library.get::<T>(name).ok().map(|found| *found) }
}

/// Turns a libheif result into `Ok` or the error to give; the message goes to the debug log.
fn check(result: &HeifError) -> io::Result<()> {
    if result.code == 0 {
        return Ok(());
    }

    if !result.message.is_null() {
        // SAFETY: libheif documents `message` as always set, a NUL-terminated string valid until the next call on the
        // object that returned it, and no such call is made before this one ends.
        let message = unsafe { CStr::from_ptr(result.message) };
        tracing::debug!(
            code = result.code,
            subcode = result.subcode,
            message = %message.to_string_lossy(),
            "libheif refused"
        );
    }

    Err(map_heif(result.code, result.subcode))
}

/// A `heif_context`, freed when it goes out of scope.
struct Context<'a> {
    api: &'a Api,
    raw: *mut c_void,
}

impl<'a> Context<'a> {
    fn new(api: &'a Api) -> io::Result<Self> {
        // SAFETY: `heif_context_alloc` takes nothing and returns a new context or NULL.
        let raw = unsafe { (api.context_alloc)() };
        if raw.is_null() {
            return Err(undecodable());
        }

        Ok(Self { api, raw })
    }
}

impl Drop for Context<'_> {
    fn drop(&mut self) {
        // SAFETY: `raw` is a context from `heif_context_alloc`, freed here once.
        unsafe { (self.api.context_free)(self.raw) }
    }
}

/// A `heif_image_handle`, released when it goes out of scope.
struct Handle<'a> {
    api: &'a Api,
    raw: *mut c_void,
}

impl Drop for Handle<'_> {
    fn drop(&mut self) {
        if self.raw.is_null() {
            return;
        }

        // SAFETY: `raw` is a handle that libheif gave out and that is released here once.
        unsafe { (self.api.handle_release)(self.raw) }
    }
}

/// A decoded `heif_image`, released when it goes out of scope.
struct Image<'a> {
    api: &'a Api,
    raw: *mut c_void,
}

impl Image<'_> {
    /// The header and the interleaved RGBA plane without the padding of its rows. The size and the stride are the
    /// image's own (the declared size of the file is not trusted for the copy) and are checked first.
    fn pixels(&self) -> io::Result<Vec<u8>> {
        let api = self.api;

        // SAFETY: `self.raw` is a live image; the channel is the one an interleaved RGBA image has.
        let (width, height) = unsafe {
            (
                (api.image_width)(self.raw, CHANNEL_INTERLEAVED),
                (api.image_height)(self.raw, CHANNEL_INTERLEAVED),
            )
        };
        let (width, height) = check_size(i64::from(width), i64::from(height))?;

        let mut stride: c_int = 0;
        // SAFETY: `self.raw` is live and `stride` is a valid place for the row length in bytes.
        let plane = unsafe { (api.plane)(self.raw, CHANNEL_INTERLEAVED, &raw mut stride) };
        if plane.is_null() {
            return Err(undecodable());
        }

        let stride = usize::try_from(stride).map_err(|_| undecodable())?;
        let row = width as usize * 4;
        let rows = height as usize;
        if stride < row {
            return Err(undecodable());
        }

        // The last row needs no padding; libheif's plane is `stride * rows` long, which this does not exceed.
        let length = (rows - 1)
            .checked_mul(stride)
            .and_then(|before| before.checked_add(row))
            .ok_or_else(undecodable)?;
        // SAFETY: libheif documents the plane as `stride` bytes per row for the image's height, which is at least
        // `length`; the memory belongs to `self.raw`, which is live and not changed while `source` is used.
        let source = unsafe { std::slice::from_raw_parts(plane, length) };

        let mut out = output(width, height)?;
        copy_rows(&mut out[HEADER..], source, row, rows, stride)?;

        Ok(out)
    }
}

impl Drop for Image<'_> {
    fn drop(&mut self) {
        if self.raw.is_null() {
            return;
        }

        // SAFETY: `raw` is an image that `heif_decode_image` gave out and that is released here once.
        unsafe { (self.api.image_release)(self.raw) }
    }
}

#[cfg(test)]
mod tests {
    use std::mem::offset_of;

    use super::{Limits, SQUARE_SIDE, lowered};

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn the_limits_struct_has_the_layout_of_the_c_header() {
        // `heif_security_limits` of libheif 1.19 and 1.20 (heif_security.h), on a 64-bit system.
        assert_eq!(offset_of!(Limits, version), 0);
        assert_eq!(offset_of!(Limits, max_image_size_pixels), 8);
        assert_eq!(offset_of!(Limits, max_number_of_tiles), 16);
        assert_eq!(offset_of!(Limits, max_bayer_pattern_pixels), 24);
        assert_eq!(offset_of!(Limits, max_items), 28);
        assert_eq!(offset_of!(Limits, max_color_profile_size), 32);
        assert_eq!(offset_of!(Limits, max_memory_block_size), 40);
        assert_eq!(offset_of!(Limits, max_components), 48);
        assert_eq!(offset_of!(Limits, max_children_per_box), 60);
        assert_eq!(offset_of!(Limits, max_total_memory), 64);
    }

    #[test]
    fn the_side_for_old_libheif_is_the_square_of_the_pixel_cap() {
        assert_eq!(SQUARE_SIDE, 8192);
    }

    #[test]
    fn a_limit_is_only_ever_lowered_and_a_disabled_one_is_set() {
        assert_eq!(lowered(0, 100), 100);
        assert_eq!(lowered(500, 100), 100);
        assert_eq!(lowered(100, 100), 100);
        assert_eq!(lowered(7, 100), 7);
    }
}
