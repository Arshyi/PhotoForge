//! The Windows clipboard, narrowly: open it, put bytes under a format, take bytes from a format.
//!
//! This is the only place the program touches the clipboard, and it holds it open for as short a
//! time as the work takes. Nothing here interprets the bytes (that is `dib.rs` and the PNG codec),
//! and nothing here reads a format it was not asked for by number.
use super::ClipboardError;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;
use windows::core::w;
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
};
use windows::Win32::System::Ole::{CF_DIB, CF_DIBV5};

/// The most bytes read out of the clipboard under any one format.
pub const MAX_CLIPBOARD_BYTES: usize = 1 << 30;

/// How many times a clipboard held by another program is tried, and how long between tries. Some
/// programs (remote-desktop clients, clipboard managers) hold it for a few milliseconds after a
/// change.
const OPEN_ATTEMPTS: u32 = 20;
const OPEN_WAIT: Duration = Duration::from_millis(15);

/// One use of the clipboard at a time within this program. A clipboard opened with no owner window
/// is not excluded by Windows (a second opener with no window succeeds), so a copy and a paste
/// that overlapped could interleave: a paste could see the clipboard between its being emptied and
/// its being filled.
static IN_PROGRAM: Mutex<()> = Mutex::new(());

/// The clipboard, open. It is closed when this is dropped, on every path.
pub struct OpenClipboardGuard {
    _alone: MutexGuard<'static, ()>,
}

impl OpenClipboardGuard {
    pub fn open() -> Result<Self, ClipboardError> {
        let alone = IN_PROGRAM
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for attempt in 0..OPEN_ATTEMPTS {
            // SAFETY: a null owner window is allowed; the clipboard belongs to this thread until
            // `CloseClipboard`, which the guard's `Drop` calls.
            if unsafe { OpenClipboard(None) }.is_ok() {
                return Ok(Self { _alone: alone });
            }
            if attempt + 1 < OPEN_ATTEMPTS {
                std::thread::sleep(OPEN_WAIT);
            }
        }
        Err(ClipboardError::Busy)
    }

    /// Empties the clipboard, which this program then owns until it closes.
    pub fn clear(&self) -> Result<(), ClipboardError> {
        // SAFETY: the clipboard is open (a guard exists).
        unsafe { EmptyClipboard() }.map_err(|e| ClipboardError::Windows(e.to_string()))
    }

    /// Gives `staged` to the clipboard under `format`. On success the system owns the memory; on
    /// failure it is freed with the value.
    pub fn set(&self, format: u32, mut staged: Staged) -> Result<(), ClipboardError> {
        let handle = staged.0.take().ok_or_else(|| {
            ClipboardError::Windows("there was nothing prepared to put on the clipboard".into())
        })?;
        // SAFETY: the clipboard is open and `handle` is a global memory block that this call
        // either hands to the system (on success) or frees here (on failure).
        match unsafe { SetClipboardData(format, Some(HANDLE(handle.0))) } {
            Ok(_) => Ok(()),
            Err(error) => {
                // SAFETY: the system did not take the block.
                let _ = unsafe { GlobalFree(Some(handle)) };
                Err(ClipboardError::Windows(error.to_string()))
            }
        }
    }

    /// Whether the clipboard holds (or can synthesise) `format`.
    pub fn has(&self, format: u32) -> bool {
        // SAFETY: a plain query.
        unsafe { IsClipboardFormatAvailable(format) }.is_ok()
    }

    /// A copy of the bytes under `format`, if there are any and they are not larger than the
    /// limit.
    pub fn get(&self, format: u32) -> Result<Option<Vec<u8>>, ClipboardError> {
        // SAFETY: the clipboard is open; the handle belongs to the clipboard and is only read
        // while locked, and the copy is made before the lock is released.
        unsafe {
            if IsClipboardFormatAvailable(format).is_err() {
                return Ok(None);
            }
            let handle = match GetClipboardData(format) {
                Ok(handle) => HGLOBAL(handle.0),
                Err(_) => return Ok(None),
            };
            let size = GlobalSize(handle);
            if size == 0 {
                return Ok(None);
            }
            if size > MAX_CLIPBOARD_BYTES {
                return Err(ClipboardError::TooLarge {
                    bytes: size as u64,
                    limit: MAX_CLIPBOARD_BYTES as u64,
                });
            }
            let source = GlobalLock(handle);
            if source.is_null() {
                return Ok(None);
            }
            let mut out = Vec::new();
            if out.try_reserve_exact(size).is_err() {
                let _ = GlobalUnlock(handle);
                return Err(ClipboardError::TooLarge {
                    bytes: size as u64,
                    limit: MAX_CLIPBOARD_BYTES as u64,
                });
            }
            out.extend_from_slice(std::slice::from_raw_parts(source.cast::<u8>(), size));
            let _ = GlobalUnlock(handle);
            Ok(Some(out))
        }
    }
}

impl Drop for OpenClipboardGuard {
    fn drop(&mut self) {
        // SAFETY: the clipboard was opened by this guard.
        let _ = unsafe { CloseClipboard() };
    }
}

/// Memory prepared for the clipboard, not yet given to it. Preparing is the part that can fail for
/// want of memory, and it is done before the clipboard is emptied; a value dropped without being
/// given away frees its memory.
pub struct Staged(Option<HGLOBAL>);

impl Drop for Staged {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            // SAFETY: the block was allocated by `stage` and never handed to the system.
            let _ = unsafe { GlobalFree(Some(handle)) };
        }
    }
}

/// Allocates global memory holding a copy of `bytes`.
pub fn stage(bytes: &[u8]) -> Result<Staged, ClipboardError> {
    // SAFETY: allocate, lock, copy exactly `bytes.len()` bytes into a block of at least that
    // size, unlock. The block is owned by the returned value until it is given away.
    unsafe {
        let handle = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1))
            .map_err(|e| ClipboardError::Windows(e.to_string()))?;
        let target = GlobalLock(handle);
        if target.is_null() {
            let _ = GlobalFree(Some(handle));
            return Err(ClipboardError::Windows(
                "the clipboard memory could not be locked".into(),
            ));
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), target.cast::<u8>(), bytes.len());
        let _ = GlobalUnlock(handle);
        Ok(Staged(Some(handle)))
    }
}

/// The registered format that tells Windows whether the content may go to the cloud clipboard (a
/// DWORD: 0 for no).
pub fn cloud_upload_format() -> u32 {
    // SAFETY: registering a name returns the same number for the same name, system-wide.
    unsafe { RegisterClipboardFormatW(w!("CanUploadToCloudClipboard")) }
}

/// The registered clipboard format called "PNG", which Office, browsers, Photoshop, GIMP and
/// Paint.NET all read and write for pictures with transparency.
pub fn png_format() -> u32 {
    // SAFETY: registering a name returns the same number for the same name, system-wide.
    unsafe { RegisterClipboardFormatW(w!("PNG")) }
}

pub fn dib_format() -> u32 {
    u32::from(CF_DIB.0)
}

pub fn dibv5_format() -> u32 {
    u32::from(CF_DIBV5.0)
}
