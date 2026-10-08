//! Faster saves, after silver2127's Big Maps for TPF2 (tpf2-bigmap,
//! `save_fast`; docs/BIGMAPS.md, "Saving").
//!
//! TF3 compresses a save with zstd at level 3 through a 128-byte stream
//! buffer. On a big map that is most of a save's time: a 45 km world's
//! 438 MB save took 7.7 s. Two instructions of the save writer's
//! `PushCompressor` are rewritten in place (profiles/…/hooks.toml, "Faster
//! saves"): the level becomes 1 and the buffer 64 KiB. On TPF2 that made
//! compression 2.79 times faster for files about 9% larger. A save is still
//! a standard zstd frame of the same bytes, so every game loads it, with or
//! without the hook; loading is not touched (`PushDecompressor` keeps
//! reading the level constant, which stays 3).
//!
//! A room never compares save files: it judges the lanes' digests
//! (docs/PROTOCOL.md, "Saving"), so games with and without this agree.
//! Each rewrite checks the site's bytes first and installs alone; a build
//! where either is not exactly as expected keeps the game's own.

#![allow(unsafe_code)]

use tpf3mp_hookcore::detour::Rewrite;

pub use crate::build_data::native::savefast::{
    BUFFER_64K_BYTES, BUFFER_BYTES, BUFFER_SIZE, LEVEL_LOAD, LEVEL_LOAD_BYTES, LEVEL_ONE_BYTES,
};

/// Opt-in: unset keeps the game's level and buffer; `1` enables the rewrite.
pub const ENV: &str = "TPF3MP_HOOK_SAVE_FAST";

fn enabled(value: Option<&str>) -> bool {
    matches!(
        value
            .map(|value| value.trim().to_ascii_lowercase())
            .as_deref(),
        Some("1" | "on" | "true" | "yes")
    )
}

/// Rewrites both sites the profile resolved, where their bytes are the
/// expected ones, and says what it did.
///
/// # Safety
///
/// `at` gives addresses in this process's image; nothing saves yet (the
/// hook installs before the game's first frame).
pub unsafe fn install(at: &dyn Fn(&str) -> Result<usize, String>) -> String {
    let setting = std::env::var(ENV).ok();
    // SAFETY: forwarded from this function's caller.
    unsafe { install_with_setting(setting.as_deref(), at) }
}

unsafe fn install_with_setting(
    setting: Option<&str>,
    at: &dyn Fn(&str) -> Result<usize, String>,
) -> String {
    if !enabled(setting) {
        return format!("faster saves: off (set {ENV}=1 to enable)");
    }
    let rewrite = |name: &str, expected: &[u8], replacement: &[u8]| {
        at(name).and_then(|address| {
            // SAFETY: the caller's; Rewrite checks the bytes before writing.
            unsafe { Rewrite::install(address as *mut u8, expected, replacement) }
                .map_err(|error| error.to_string())
        })
    };
    let level = rewrite(LEVEL_LOAD, &LEVEL_LOAD_BYTES, &LEVEL_ONE_BYTES);
    let buffer = rewrite(BUFFER_SIZE, &BUFFER_BYTES, &BUFFER_64K_BYTES);
    let level = match level {
        Ok(patch) => {
            // Keep it for the process lifetime. ManuallyDrop works on
            // unsupported architectures too, where Rewrite has no Drop.
            let _kept = std::mem::ManuallyDrop::new(patch);
            "zstd level 1".to_owned()
        }
        Err(why) => format!("the game's zstd level ({why})"),
    };
    let buffer = match buffer {
        Ok(patch) => {
            // Keep it for the process lifetime. ManuallyDrop works on
            // unsupported architectures too, where Rewrite has no Drop.
            let _kept = std::mem::ManuallyDrop::new(patch);
            "a 64 KiB buffer".to_owned()
        }
        Err(why) => format!("the game's 128-byte buffer ({why})"),
    };
    format!("faster saves: {level}, {buffer} (opt-in via {ENV}=1)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_rewrites_are_opt_in_and_explicit_disable_still_wins() {
        assert!(
            !enabled(None),
            "unset must preserve the native save settings"
        );
        for value in ["0", "off", "false", "no", "", "unknown", "2", "  "] {
            assert!(!enabled(Some(value)), "{value} must disable the rewrite");
        }
        for value in ["1", "on", "true", "yes"] {
            assert!(enabled(Some(value)), "{value} must explicitly enable it");
        }
        assert!(
            enabled(Some(" YES ")),
            "explicit values are trimmed and folded"
        );
    }

    #[test]
    fn install_logs_the_opt_in_when_disabled_without_resolving_sites() {
        // SAFETY: no site is resolved or changed on the disabled path.
        let line = unsafe {
            install_with_setting(None, &|name| Err(format!("unexpected lookup: {name}")))
        };
        assert_eq!(
            line,
            "faster saves: off (set TPF3MP_HOOK_SAVE_FAST=1 to enable)"
        );
    }

    #[test]
    fn the_rewrites_keep_each_instruction_whole() {
        // mov eax,imm32; nop in place of mov eax,[rip+disp32].
        assert_eq!(LEVEL_LOAD_BYTES[..2], [0x8B, 0x05]);
        assert_eq!(LEVEL_ONE_BYTES, [0xB8, 1, 0, 0, 0, 0x90]);
        // mov r8d,imm32, its immediate 0x80 then 0x10000.
        assert_eq!(BUFFER_BYTES[..2], BUFFER_64K_BYTES[..2]);
        assert_eq!(
            u32::from_le_bytes(BUFFER_BYTES[2..].try_into().unwrap()),
            0x80
        );
        assert_eq!(
            u32::from_le_bytes(BUFFER_64K_BYTES[2..].try_into().unwrap()),
            0x1_0000
        );
    }

    #[test]
    fn missing_sites_leave_the_game_its_own() {
        // SAFETY: both lookups fail, so no address is written.
        let line = unsafe {
            install_with_setting(Some("1"), &|name| {
                Err(format!("{name} is not in this build"))
            })
        };
        assert!(line.contains("the game's zstd level"), "{line}");
        assert!(line.contains("the game's 128-byte buffer"), "{line}");
    }

    #[test]
    fn the_profile_names_both_sites_with_the_bytes_rewritten() {
        let profile =
            tpf3mp_hookcore::profile::Profile::from_toml(crate::build_data::native::PROFILE_TOML)
                .unwrap();
        for (name, bytes) in [
            (LEVEL_LOAD, &LEVEL_LOAD_BYTES),
            (BUFFER_SIZE, &BUFFER_BYTES),
        ] {
            let target = profile
                .targets
                .iter()
                .find(|target| target.name == name)
                .unwrap_or_else(|| panic!("{name} is in the profile"));
            assert!(!target.required, "{name} is optional");
            assert_eq!(&target.prologue[..], &bytes[..], "{name}");
        }
    }
}
