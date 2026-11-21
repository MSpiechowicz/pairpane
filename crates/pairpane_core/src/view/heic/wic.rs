//! HEIC through the Windows Imaging Component, whose HEIF codec is the user's HEIF Image Extensions and whose HEVC
//! decoder is the HEVC Video Extensions; without them the component reports that no codec is found and the picture
//! is `Unsupported`.
//!
//! The component converts the first frame to 32-bit RGBA (not premultiplied) and copies it into the output buffer in
//! one call. Known limitation: the component hands out the stored pixels, so a HEIC whose rotation or mirroring is
//! stored in its transformation properties (a photo taken sideways) is shown unrotated. libheif applies them.
//!
//! The decoder is made for the HEIF container only (never chosen by sniffing the bytes), so no other codec of the
//! system parses the file. The component runs only in the helper process that decodes a picture (see `process.rs`), so
//! a crash of a codec on a hostile file does not reach the app.
//!
//! COM is entered on the calling thread (multi-threaded apartment) and left again when the picture is done, unless the
//! thread was in another mode already, which is then left alone. Every COM object is released before COM is left.
#![allow(
    unsafe_code,
    reason = "COM calls of the Windows Imaging Component; see `decode` and `convert`"
)]
#![deny(clippy::undocumented_unsafe_blocks)]

use std::io;
use std::ptr;

use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_ContainerFormatHeif, GUID_WICPixelFormat32bppRGBA, IWICImagingFactory, IWICPalette,
    WICBitmapDitherTypeNone, WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};

use super::{HEADER, MAX_IMAGE_BYTES, check_size, map_hresult, output, too_big};

/// Keeps COM entered on this thread for as long as it lives.
struct Com {
    /// Whether this guard entered COM (and so leaves it); not when the thread was in another mode already.
    entered: bool,
}

impl Com {
    fn enter() -> io::Result<Self> {
        // SAFETY: `CoInitializeEx` takes no pointer here (reserved is NULL) and only changes this thread's COM state.
        let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };

        // S_OK and S_FALSE (already entered in this mode) both count a call that `CoUninitialize` must undo.
        if result.is_ok() {
            return Ok(Self { entered: true });
        }

        // The thread is in single-threaded mode already (the page's own thread, say); COM works there as well.
        if result == RPC_E_CHANGED_MODE {
            return Ok(Self { entered: false });
        }

        Err(map_hresult(result.0))
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if self.entered {
            // SAFETY: balances the successful `CoInitializeEx` of `Com::enter` on this same thread, once.
            unsafe { CoUninitialize() }
        }
    }
}

/// The picture of the HEIC file `bytes` as the header and the RGBA pixels (see the parent module).
pub(super) fn decode(bytes: &[u8]) -> io::Result<Vec<u8>> {
    // The stream below takes a 32-bit length.
    if bytes.len() as u64 > MAX_IMAGE_BYTES {
        return Err(too_big());
    }

    let _com = Com::enter()?;

    // Every COM object lives inside `convert`, so all are released before `_com` leaves COM.
    FAILED_AT.set(None);
    match convert(bytes) {
        Ok(result) => result,
        Err(error) => {
            let step = FAILED_AT.get().map_or("?", |(step, _)| step);
            tracing::debug!(%error, step, "the imaging component refused");

            // No HEIF decoder can be made at all: the HEIF extension is not installed (whatever the code says).
            if step == "CreateDecoder" {
                return Err(super::no_decoder());
            }

            Err(map_hresult(error.code().0))
        }
    }
}

thread_local! {
    /// The step of the last failed conversion on this thread and its `HRESULT`, for the log and the tests.
    static FAILED_AT: std::cell::Cell<Option<(&'static str, i32)>> = const { std::cell::Cell::new(None) };
}

/// The step of the last failed decode on this thread and its `HRESULT` (for a test to say what Windows answered).
#[cfg(test)]
pub(super) fn failed_at() -> Option<(&'static str, i32)> {
    FAILED_AT.get()
}

/// Notes `step` as the one that failed when `result` is an error.
fn at<T>(step: &'static str, result: windows::core::Result<T>) -> windows::core::Result<T> {
    if let Err(error) = &result {
        FAILED_AT.set(Some((step, error.code().0)));
    }

    result
}

/// The conversion itself. The outer error is a COM failure to be mapped; the inner result is a refusal of our own.
fn convert(bytes: &[u8]) -> windows::core::Result<io::Result<Vec<u8>>> {
    // SAFETY: the class and the flags are the documented way to get the imaging factory in this process, and the
    // result is an owned interface that releases itself.
    let factory: IWICImagingFactory = unsafe {
        at(
            "CoCreateInstance",
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER),
        )?
    };

    // SAFETY: `factory` is a live interface. `bytes` is valid for its length and outlives `stream` and everything
    // made from it (it is a parameter of this function, and they are all dropped before it returns); the stream reads
    // it without copying and never writes it.
    let stream = unsafe {
        let stream = at("CreateStream", factory.CreateStream())?;
        at("InitializeFromMemory", stream.InitializeFromMemory(bytes))?;

        stream
    };

    // SAFETY: `factory` and `stream` are live; the container is the HEIF one (a constant GUID) and the vendor is NULL
    // (any HEIF decoder), so only the HEIF codec reads the stream; the decoder, the frame and its size are owned
    // interfaces and values made from them.
    let (frame, width, height) = unsafe {
        let decoder = at(
            "CreateDecoder",
            factory.CreateDecoder(&GUID_ContainerFormatHeif, ptr::null()),
        )?;
        at(
            "Initialize",
            decoder.Initialize(&stream, WICDecodeMetadataCacheOnDemand),
        )?;
        let frame = at("GetFrame", decoder.GetFrame(0))?;
        let (mut width, mut height) = (0, 0);
        at("GetSize", frame.GetSize(&raw mut width, &raw mut height))?;

        (frame, width, height)
    };

    // The size the decoder reports is checked before the converter is made and anything is allocated for it.
    let (width, height) = match check_size(i64::from(width), i64::from(height)) {
        Ok(size) => size,
        Err(error) => return Ok(Err(error)),
    };
    let mut out = match output(width, height) {
        Ok(out) => out,
        Err(error) => return Ok(Err(error)),
    };

    // SAFETY: `factory` and `frame` are live. The converter reads only `frame`; the destination format is a constant
    // GUID, the palette is absent (none is needed for a true-colour format) and the stride and the buffer are those of
    // the checked size: `width * 4` bytes per row (at most 131 068) and exactly `width * height * 4` bytes after the
    // header. `CopyPixels` gets a NULL rectangle, which means the whole frame, and writes only inside `out[HEADER..]`.
    unsafe {
        let converter = at("CreateFormatConverter", factory.CreateFormatConverter())?;
        let initialized = converter.Initialize(
            &frame,
            &GUID_WICPixelFormat32bppRGBA,
            WICBitmapDitherTypeNone,
            None::<&IWICPalette>,
            0.0,
            WICBitmapPaletteTypeCustom,
        );
        at("FormatConverter::Initialize", initialized)?;
        at(
            "CopyPixels",
            converter.CopyPixels(ptr::null(), width * 4, &mut out[HEADER..]),
        )?;
    }

    Ok(Ok(out))
}
