//! The system clipboard, for the Multiplayer windows' Copy beside a room's
//! invite code. The game's GUI has no clipboard call of its own (none in
//! `api/tealdef` or its GUI scripts, build 40408), so the hook sets it: on
//! Windows through the system's clipboard, elsewhere through the SDL the
//! game runs on (`SDL_SetClipboardText`, found in the game's process).

#![allow(unsafe_code)]

/// The longest text copied, in characters: an invite is far shorter.
pub const MAX_CHARS: usize = 1024;

/// Puts `text` on the clipboard, or says why not.
pub fn copy(text: &str) -> Result<(), String> {
    let text = checked(text)?;
    platform::copy(text)
}

/// The text to copy: not empty, not too long, without NULs.
fn checked(text: &str) -> Result<&str, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("nothing to copy".into());
    }
    if text.chars().count() > MAX_CHARS || text.contains('\0') {
        return Err("that cannot be copied".into());
    }
    Ok(text)
}

/// `text` as Windows wants it on the clipboard: UTF-16, NUL-ended.
#[cfg_attr(not(windows), allow(dead_code))]
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
mod platform {
    use windows_sys::Win32::{
        Foundation::GlobalFree,
        System::{
            DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData},
            Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock},
        },
    };

    /// The clipboard's format for UTF-16 text (`CF_UNICODETEXT`).
    const CF_UNICODETEXT: u32 = 13;

    pub fn copy(text: &str) -> Result<(), String> {
        let wide = super::wide(text);
        let bytes = wide.len() * 2;
        // SAFETY: the documented sequence: open the clipboard for this
        // thread, empty it, hand it a moveable global block of the text
        // (the system owns it once SetClipboardData succeeds; freed here
        // otherwise), close it.
        unsafe {
            if OpenClipboard(std::ptr::null_mut()) == 0 {
                return Err("the clipboard is busy".into());
            }
            let result = (|| {
                if EmptyClipboard() == 0 {
                    return Err("the clipboard cannot be emptied".to_owned());
                }
                let block = GlobalAlloc(GMEM_MOVEABLE, bytes);
                if block.is_null() {
                    return Err("no memory for the clipboard".to_owned());
                }
                let target = GlobalLock(block).cast::<u16>();
                if target.is_null() {
                    GlobalFree(block);
                    return Err("no memory for the clipboard".to_owned());
                }
                std::ptr::copy_nonoverlapping(wide.as_ptr(), target, wide.len());
                GlobalUnlock(block);
                if SetClipboardData(CF_UNICODETEXT, block).is_null() {
                    GlobalFree(block);
                    return Err("the clipboard did not take it".to_owned());
                }
                Ok(())
            })();
            CloseClipboard();
            result
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use std::ffi::{CString, c_char, c_int, c_void};

    unsafe extern "C" {
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }

    /// `RTLD_DEFAULT`: every library the process has loaded.
    #[cfg(target_os = "macos")]
    const RTLD_DEFAULT: *mut c_void = -2isize as *mut c_void;
    #[cfg(not(target_os = "macos"))]
    const RTLD_DEFAULT: *mut c_void = std::ptr::null_mut();

    type SetClipboardText = unsafe extern "C" fn(*const c_char) -> c_int;

    pub fn copy(text: &str) -> Result<(), String> {
        let text = CString::new(text).map_err(|_| "that cannot be copied".to_owned())?;
        // SAFETY: a lookup by name in the process; the symbol, when there,
        // is SDL2's `int SDL_SetClipboardText(const char *)`, called on the
        // GUI's thread as SDL wants.
        unsafe {
            let found = dlsym(RTLD_DEFAULT, c"SDL_SetClipboardText".as_ptr());
            if found.is_null() {
                return Err("the game's SDL has no clipboard".into());
            }
            let set = std::mem::transmute::<*mut c_void, SetClipboardText>(found);
            if set(text.as_ptr()) == 0 {
                Ok(())
            } else {
                Err("the clipboard did not take it".into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_short_text_is_copied_as_utf16() {
        assert_eq!(checked("  K7QM2X \n"), Ok("K7QM2X"));
        assert!(checked("   ").is_err());
        assert!(checked("a\0b").is_err());
        assert!(checked(&"x".repeat(MAX_CHARS + 1)).is_err());
        assert_eq!(wide("Kä"), [u16::from(b'K'), 0xe4, 0]);
    }

    /// Sets the real clipboard: run by hand.
    #[test]
    #[ignore = "changes the clipboard of whoever runs the tests"]
    fn the_clipboard_takes_an_invite() {
        copy("K7QM2X").unwrap();
    }
}
