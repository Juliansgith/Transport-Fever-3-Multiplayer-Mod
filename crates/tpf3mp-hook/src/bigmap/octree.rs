//! The octree root at depth 11 for worlds with an axis over 256 tiles.
//!
//! TF3 sizes its entity octree from a two-tier constant, not from the map:
//! ±16,384 m at depth 9, and, when either axis is over 128 tiles,
//! ±32,768 m at depth 10, set by `OctreeSystem::Resize` at two sites, one
//! on the new-game path and one on the load path. An entity past the root
//! is invisible to lookups, and the street builder stacks duplicate nodes
//! there, so a world with an axis over 256 tiles (65.5 km) damages itself
//! (investigation/TF3_BIGMAPS_PORT_2026-10-01.md §1.3).
//!
//! With [`OCTREE_ENV`] set to `11`, the instruction after each site's call
//! is spliced: when an axis of the world being allocated is over 256
//! tiles, the hook calls the game's own `Resize` again, with depth 11 and
//! ±65,536 m. That is tpf2-bigmap's depth-11 root (silver2127, MIT; no code
//! taken), reached without patching the shared 32768.0f constant or any
//! instruction but the splice's jump. Depth 11 keeps the 128 m leaves, and
//! its deepest level's ids end at 1,227,133,512, inside the stock id scheme
//! and the renderer's level decoder (investigation/
//! TF3_BIGMAPS_256KM_2026-10-05.md §1). A world over 512 tiles needs depth
//! 12, which is not built: it is left at the game's depth, with a line in
//! `hook.log`, and the New Game page must not offer it.
//!
//! A world of 256 tiles or less is never touched, so a game with the
//! setting and one without agree on every stock-sized world. On a world
//! over 256 tiles every game of a room needs the setting.

#![allow(unsafe_code)]

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};

use tpf3mp_hookcore::detour::{SavedRegs, Splice};
use tpf3mp_hookcore::profile::ResolvedProfile;

pub use crate::build_data::native::bigmap::{
    INIT_SITE, LOAD_SITE, OCTREE_FIELD, RESIZE, ROOT_CALL_OFFSET, ROOT_DEPTH_BYTES,
    ROOT_NEXT_OFFSET, ROOT_OWNER_OFFSET, Reg, RootSite,
};
use crate::log;

/// The patch's name in `hook.log`.
pub const FIX: &str = "big maps: octree";

/// `11` in the game's environment moves the root to depth 11 for worlds
/// with an axis over 256 tiles; unset or `0` leaves the game's own.
pub const OCTREE_ENV: &str = "TPF3MP_BIGMAP_OCTREE";

/// The site's own test: a root of depth 10 above this many tiles.
pub const STOCK_LARGE_TILES: i32 = 128;
/// The longest axis the stock depth-10 root covers: ±32,768 m.
pub const STOCK_ROOT_TILES: i32 = 256;
/// The longest axis depth 11 covers: ±65,536 m.
pub const DEPTH11_TILES: i32 = 512;
/// Depth 11's depth and half extent.
pub const DEPTH11: (i32, f32) = (11, 65_536.0);
/// The longest axis depth 12 covers: ±131,072 m.
pub const DEPTH12_TILES: i32 = 1024;
/// Depth 12's depth and half extent.
pub const DEPTH12: (i32, f32) = (12, 131_072.0);

/// What the setting asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setting {
    /// The game's own root.
    Off,
    /// Depth 11 for worlds with an axis over 256 tiles.
    Depth11,
    /// Depth 11 over 256 tiles and depth 12 (crate::bigmap::depth12) over
    /// 512.
    Depth12,
}

/// The setting from [`OCTREE_ENV`]'s value; anything but `11`, `0` or
/// unset is refused, so a typo never applies a root nobody asked for.
pub fn setting(value: Option<&str>) -> Result<Setting, String> {
    match value.map(str::trim) {
        None | Some("" | "0") => Ok(Setting::Off),
        Some("11") => Ok(Setting::Depth11),
        Some("12") => Ok(Setting::Depth12),
        Some("13") => Err(format!("{OCTREE_ENV}=13 is not built on TF3 (11 or 12)")),
        Some(other) => Err(format!("{OCTREE_ENV}={other:?} is not 12, 11 or 0")),
    }
}

/// What a world of `x` by `y` tiles gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Root {
    /// The game's own root, as its site set it.
    Stock,
    /// Depth 11, ±65,536 m.
    Depth11,
    /// Depth 12, ±131,072 m.
    Depth12,
    /// Longer than the setting covers: left at the game's own, which damages
    /// the world.
    Beyond { tiles: i32 },
}

/// The root a world of `x` by `y` tiles gets with the patch set to
/// `setting`: the game's own up to 256 tiles, depth 11 up to 512, depth 12
/// up to 1,024 when set to 12, and past that nothing this hook can give.
pub fn root_for(x: i32, y: i32, setting: Setting) -> Root {
    let longest = x.max(y);
    if setting == Setting::Off || longest <= STOCK_ROOT_TILES {
        Root::Stock
    } else if longest <= DEPTH11_TILES {
        Root::Depth11
    } else if longest <= DEPTH12_TILES && setting == Setting::Depth12 {
        Root::Depth12
    } else {
        Root::Beyond { tiles: longest }
    }
}

/// `OctreeSystem::Resize(octree, depth, half)`.
type ResizeFn = unsafe extern "system" fn(usize, i32, f32);

/// `Resize`'s address while the patch is on; 0 otherwise.
static RESIZE_AT: AtomicUsize = AtomicUsize::new(0);
/// The setting the splices act on: 0 off, 11 or 12.
static INSTALLED: AtomicI32 = AtomicI32::new(0);
/// Set when the hook panicked: it does nothing more.
static BROKEN: AtomicBool = AtomicBool::new(false);

/// A register of the splice's block.
fn reg(regs: &SavedRegs, which: Reg) -> u64 {
    match which {
        Reg::Rbp => regs.rbp,
        Reg::Rsi => regs.rsi,
    }
}

fn read<T: Copy>(address: u64) -> Option<T> {
    let address = usize::try_from(address).ok()?;
    if address == 0 || !crate::image::readable(address, std::mem::size_of::<T>()) {
        return None;
    }
    // SAFETY: the bytes are readable, checked just above.
    Some(unsafe { std::ptr::read_unaligned(address as *const T) })
}

/// After a site's `Resize`: the world's tile counts and the octree, from
/// the site's registers; depth 11 where the world needs it.
fn after_resize(regs: &SavedRegs, site: &RootSite) {
    let tiles = reg(regs, site.tiles);
    let (Some(x), Some(y)) = (read::<i32>(tiles), read::<i32>(tiles.wrapping_add(4))) else {
        log::line(&format!(
            "{FIX}: {}: the tile counts at {tiles:#x} are unreadable; the game's root stays",
            site.target
        ));
        return;
    };
    let setting = match INSTALLED.load(Ordering::Acquire) {
        12 => Setting::Depth12,
        11 => Setting::Depth11,
        _ => Setting::Off,
    };
    let root = root_for(x, y, setting);
    let (depth, half) = if root == Root::Depth12 {
        DEPTH12
    } else {
        DEPTH11
    };
    match root {
        Root::Stock => {}
        Root::Beyond { .. } => log::line(&format!(
            "{FIX}: {}: a world of {x} x {y} tiles is past what {OCTREE_ENV} reaches; the game's root (±32,768 m) stays and entities past it will be lost",
            site.target
        )),
        Root::Depth11 | Root::Depth12 => {
            let owner = reg(regs, site.owner);
            let octree = read::<usize>(owner.wrapping_add(OCTREE_FIELD as u64)).unwrap_or(0);
            let resize = RESIZE_AT.load(Ordering::Acquire);
            if octree == 0 || resize == 0 || !crate::image::readable(octree, 0x2c) {
                log::line(&format!(
                    "{FIX}: {}: no octree at {owner:#x}+{OCTREE_FIELD:#x}; the game's root stays",
                    site.target
                ));
                return;
            }
            // SAFETY: the game's own Resize (the profile resolved it, and
            // install checked each site calls it), on the octree the site
            // just resized, on the game's thread, before anything is in it:
            // it only stores the box and the depth.
            unsafe { std::mem::transmute::<usize, ResizeFn>(resize)(octree, depth, half) };
            log::line(&format!(
                "{FIX}: {}: a world of {x} x {y} tiles; root at depth {depth}, ±{half} m",
                site.target
            ));
        }
    }
}

fn guarded(regs: *mut SavedRegs, site: &RootSite) {
    if BROKEN.load(Ordering::Acquire) {
        return;
    }
    let body = || {
        // SAFETY: the stub's block, held until the hook returns.
        let regs = unsafe { &*regs };
        after_resize(regs, site);
    };
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)).is_err() {
        BROKEN.store(true, Ordering::Release);
        log::line(&format!(
            "{FIX}: panicked on the game's thread; switched off for this game"
        ));
    }
}

unsafe extern "system" fn init_hook(regs: *mut SavedRegs) {
    guarded(regs, &INIT_SITE);
}

unsafe extern "system" fn load_hook(regs: *mut SavedRegs) {
    guarded(regs, &LOAD_SITE);
}

/// The bytes at a site must be the block the patch was read on: the owner
/// load, `mov edx,10`, a `call` of `Resize`, and the instruction to splice.
pub fn check_site(
    bytes: &[u8],
    site_at: u64,
    resize_at: u64,
    site: &RootSite,
) -> Result<(), String> {
    let end = ROOT_NEXT_OFFSET + site.next.len();
    if bytes.len() < end {
        return Err(format!("{}: only {} bytes", site.target, bytes.len()));
    }
    let owner = &bytes[ROOT_OWNER_OFFSET..ROOT_OWNER_OFFSET + 4];
    if owner != site.owner_load {
        return Err(format!(
            "{}: the owner load is not the expected one",
            site.target
        ));
    }
    if bytes[ROOT_OWNER_OFFSET + 4..ROOT_CALL_OFFSET] != ROOT_DEPTH_BYTES {
        return Err(format!("{}: no `mov edx, 10` before the call", site.target));
    }
    if bytes[ROOT_CALL_OFFSET] != 0xE8 {
        return Err(format!("{}: no call at +{ROOT_CALL_OFFSET}", site.target));
    }
    let rel = i32::from_le_bytes(
        bytes[ROOT_CALL_OFFSET + 1..ROOT_NEXT_OFFSET]
            .try_into()
            .map_err(|_| "a short call".to_owned())?,
    );
    let callee = (site_at + ROOT_NEXT_OFFSET as u64).wrapping_add_signed(i64::from(rel));
    if callee != resize_at {
        return Err(format!(
            "{}: the call reaches {callee:#x}, not {RESIZE} at {resize_at:#x}",
            site.target
        ));
    }
    if bytes[ROOT_NEXT_OFFSET..end] != site.next {
        return Err(format!(
            "{}: the instruction after the call is not the expected one",
            site.target
        ));
    }
    Ok(())
}

/// What installing came to, for `hook.log`.
pub fn outcome_line(installed: bool, reason: &str) -> String {
    if installed {
        format!("{FIX}: installed ({reason})")
    } else {
        format!("{FIX}: off, {reason}")
    }
}

/// Installs the patch if [`OCTREE_ENV`] asks for it. Returns the line for
/// `hook.log`.
pub fn install(resolved: &ResolvedProfile) -> String {
    match setting(std::env::var(OCTREE_ENV).ok().as_deref()) {
        Ok(wanted) => install_with(resolved, wanted),
        Err(why) => outcome_line(false, &format!("{why}; the game's own root")),
    }
}

/// [`install`] with the setting given. The splices are kept for the life
/// of the game.
pub fn install_with(resolved: &ResolvedProfile, wanted: Setting) -> String {
    // Depth 12's ids and decoder go in first; the roots that use them last.
    let deep = if wanted == Setting::Depth12 {
        match super::depth12::install(resolved) {
            Ok(deep) => Some(deep),
            Err(why) => return outcome_line(false, &format!("depth 12: {why}")),
        }
    } else {
        None
    };
    match install_splices(resolved, wanted) {
        Ok(splices) => {
            let reach = if deep.is_some() {
                format!(
                    "depth 11 (±65,536 m) for worlds with an axis over {STOCK_ROOT_TILES} tiles and depth 12 (±131,072 m, deep ids by cell) over {DEPTH11_TILES}"
                )
            } else {
                format!(
                    "depth 11, ±65,536 m, for worlds with an axis over {STOCK_ROOT_TILES} tiles"
                )
            };
            let setting = if deep.is_some() { 12 } else { 11 };
            let line = outcome_line(
                true,
                &format!(
                    "{reach}, at {} sites; every game of a room on such a world needs {OCTREE_ENV}={setting}",
                    splices.len()
                ),
            );
            let _kept = std::mem::ManuallyDrop::new((splices, deep));
            line
        }
        Err(why) => {
            if let Some(deep) = deep {
                // SAFETY: nothing runs the game's code yet.
                let _ = unsafe { deep.descent.detach() };
                let _ = unsafe { deep.decoder.detach() };
            }
            outcome_line(false, &why)
        }
    }
}

/// Checks both sites and splices both, or neither.
pub(crate) fn install_splices(
    resolved: &ResolvedProfile,
    wanted: Setting,
) -> Result<Vec<Splice>, String> {
    if wanted == Setting::Off {
        return Err(format!(
            "{OCTREE_ENV} is not set to 11 or 12; the game's own root"
        ));
    }
    let resize = resolved
        .get(RESIZE)
        .ok_or_else(|| format!("the profile has no {RESIZE:?}"))?
        .address;
    let sites = [
        (
            INIT_SITE,
            init_hook as unsafe extern "system" fn(*mut SavedRegs),
        ),
        (LOAD_SITE, load_hook),
    ];
    let mut found = Vec::new();
    for (site, hook) in sites {
        let at = resolved
            .get(site.target)
            .ok_or_else(|| format!("the profile has no {:?}", site.target))?
            .address;
        let len = ROOT_NEXT_OFFSET + site.next.len();
        let address = usize::try_from(at).map_err(|_| "an address past usize".to_owned())?;
        if !crate::image::readable(address, len) {
            return Err(format!("{} at {at:#x} is unreadable", site.target));
        }
        // SAFETY: `len` readable bytes, checked just above.
        let bytes = unsafe { std::slice::from_raw_parts(address as *const u8, len) };
        check_site(bytes, at, resize, &site)?;
        found.push((address + ROOT_NEXT_OFFSET, site, hook));
    }
    RESIZE_AT.store(resize as usize, Ordering::Release);
    INSTALLED.store(
        if wanted == Setting::Depth12 { 12 } else { 11 },
        Ordering::Release,
    );
    let mut splices = Vec::new();
    for (next, site, hook) in found {
        // SAFETY: the instruction after the site's call, checked to be the
        // 5-byte `vmovss xmm2,[reg+0x20]` the patch was read on; no world
        // is allocated yet (the hook installs before the game runs), and
        // the only branch to it (the site's `jle`) lands on its first byte.
        match unsafe { Splice::install(next as *mut u8, &site.next, site.next.len(), hook) } {
            Ok(splice) => splices.push(splice),
            Err(error) => {
                for splice in splices {
                    // SAFETY: as for install; nothing runs the site yet.
                    let _ = unsafe { splice.detach() };
                }
                RESIZE_AT.store(0, Ordering::Release);
                INSTALLED.store(0, Ordering::Release);
                return Err(format!("{} at {next:#x}: {error}", site.target));
            }
        }
    }
    Ok(splices)
}

#[cfg(test)]
pub(crate) fn reset() {
    RESIZE_AT.store(0, Ordering::Release);
    INSTALLED.store(0, Ordering::Release);
    BROKEN.store(false, Ordering::Release);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_11_turns_it_on() {
        assert_eq!(setting(None), Ok(Setting::Off));
        assert_eq!(setting(Some("")), Ok(Setting::Off));
        assert_eq!(setting(Some("0")), Ok(Setting::Off));
        assert_eq!(setting(Some(" 11 ")), Ok(Setting::Depth11));
        assert_eq!(setting(Some("12")), Ok(Setting::Depth12));
        assert!(setting(Some("13")).unwrap_err().contains("not built"));
        assert!(setting(Some("on")).is_err());
    }

    #[test]
    fn only_worlds_past_256_tiles_move_the_root() {
        assert_eq!(
            root_for(112, 112, Setting::Depth11),
            Root::Stock,
            "Gigantomaniac"
        );
        assert_eq!(
            root_for(50, 250, Setting::Depth11),
            Root::Stock,
            "Gigantomaniac 1:5"
        );
        assert_eq!(
            root_for(176, 176, Setting::Depth11),
            Root::Stock,
            "Stage 1's largest"
        );
        assert_eq!(
            root_for(256, 2, Setting::Depth11),
            Root::Stock,
            "exactly the stock root"
        );
        assert_eq!(root_for(258, 2, Setting::Depth11), Root::Depth11);
        assert_eq!(root_for(60, 300, Setting::Depth11), Root::Depth11);
        assert_eq!(root_for(512, 512, Setting::Depth11), Root::Depth11);
        assert_eq!(
            root_for(186, 1000, Setting::Depth11),
            Root::Beyond { tiles: 1000 }
        );
        assert_eq!(
            root_for(514, 2, Setting::Depth11),
            Root::Beyond { tiles: 514 }
        );
        // Set to 12: the same up to 512, depth 12 up to 1,024.
        for (x, y, root) in [
            (176, 176, Root::Stock),
            (60, 300, Root::Depth11),
            (512, 512, Root::Depth11),
            (186, 1000, Root::Depth12),
            (2, 1024, Root::Depth12),
            (2, 1026, Root::Beyond { tiles: 1026 }),
        ] {
            assert_eq!(root_for(x, y, Setting::Depth12), root, "{x} x {y}");
        }
        assert_eq!(root_for(186, 1000, Setting::Off), Root::Stock);
        assert_eq!(f64::from(DEPTH12.1), f64::from(DEPTH12_TILES) * 128.0);
    }

    #[test]
    fn depth_11_covers_512_tiles_of_256_m() {
        let (depth, half) = DEPTH11;
        assert_eq!(depth, 11);
        assert_eq!(f64::from(half), f64::from(DEPTH11_TILES) * 256.0 / 2.0);
        assert_eq!(f64::from(STOCK_ROOT_TILES) * 256.0 / 2.0, 32_768.0);
        // Depth 11's deepest level, 10, ends at (8^11 - 1) / 7 - 1, a
        // positive int32: the stock ids and the decoder hold.
        let last: u64 = (8u64.pow(11) - 1) / 7 - 1;
        assert_eq!(last, 0x4924_9248);
        assert!(last < i32::MAX as u64);
        assert!(
            (8u64.pow(12) - 1) / 7 - 1 > u32::MAX as u64,
            "depth 12 does not fit"
        );
    }

    fn site_bytes(site: &RootSite, at: u64, resize: u64) -> Vec<u8> {
        let mut bytes = vec![0xC5, 0xFA, 0x10, 0x15, 0, 0, 0, 0];
        bytes.extend_from_slice(&site.owner_load);
        bytes.extend_from_slice(&ROOT_DEPTH_BYTES);
        let rel = (resize as i64 - (at as i64 + ROOT_NEXT_OFFSET as i64)) as i32;
        bytes.push(0xE8);
        bytes.extend_from_slice(&rel.to_le_bytes());
        bytes.extend_from_slice(&site.next);
        bytes
    }

    #[test]
    fn a_site_is_checked_byte_for_byte() {
        for site in [INIT_SITE, LOAD_SITE] {
            let bytes = site_bytes(&site, 0x244b8b, 0xae4c50);
            assert_eq!(check_site(&bytes, 0x244b8b, 0xae4c50, &site), Ok(()));
            assert!(
                check_site(&bytes, 0x244b8b, 0xae4c60, &site).is_err(),
                "another callee"
            );
            assert!(check_site(&bytes[..20], 0x244b8b, 0xae4c50, &site).is_err());
            for at in [8, 13, 17, 22, 26] {
                let mut changed = bytes.clone();
                changed[at] ^= 0x01;
                assert!(
                    check_site(&changed, 0x244b8b, 0xae4c50, &site).is_err(),
                    "byte {at}"
                );
            }
        }
        // The two sites keep their registers apart.
        let init = site_bytes(&INIT_SITE, 0, 0x100);
        assert!(check_site(&init, 0, 0x100, &LOAD_SITE).is_err());
    }

    #[test]
    fn the_release_profile_names_the_sites_bytes() {
        let profile =
            tpf3mp_hookcore::profile::Profile::from_toml(crate::BUILT_IN_PROFILES[0].1).unwrap();
        let target = |name: &str| {
            profile
                .targets
                .iter()
                .find(|t| t.name == name)
                .unwrap_or_else(|| panic!("{name} in the profile"))
        };
        // The release build's own addresses (RVAs): each site's prologue
        // passes the check against Resize there.
        for (site, at) in [(INIT_SITE, 0x244b8b), (LOAD_SITE, 0x20267d)] {
            let spec = target(site.target);
            assert!(!spec.required);
            assert_eq!(check_site(&spec.prologue, at, 0xae4c50, &site), Ok(()));
        }
        assert!(!target(RESIZE).required);
    }

    #[test]
    fn nothing_installs_when_off_or_without_the_targets() {
        let empty = ResolvedProfile {
            name: "empty".into(),
            targets: Vec::new(),
            absent_optional: Vec::new(),
        };
        let line = install_with(&empty, Setting::Off);
        assert!(line.starts_with("big maps: octree: off"), "{line}");
        assert!(line.contains(OCTREE_ENV), "{line}");
        let line = install_with(&empty, Setting::Depth11);
        assert!(line.contains("the profile has no"), "{line}");
    }
}

/// The game's own `Resize` and both root sites, relocated from the
/// executable and run with the splices installed by [`install_splices`]
/// (crate::bigmap::original).
#[cfg(all(test, windows, target_arch = "x86_64"))]
#[allow(clippy::unwrap_used)]
mod original_tests {
    use std::sync::Mutex;

    use tpf3mp_hookcore::profile::ResolvedTarget;

    use super::*;
    use crate::bigmap::original::{Exe, Page};

    static SERIAL: Mutex<()> = Mutex::new(());

    const RESIZE_RVA: u64 = 0xae4c50;
    const RESIZE_LEN: usize = 101;
    /// The empty profiling scope `Resize` calls twice.
    const SCOPE_RVA: u64 = 0x55b50;
    /// The sign mask `Resize` negates the half with.
    const SIGN_RVA: u64 = 0x3676a10;
    /// The sites' 32768.0f.
    const HALF_RVA: u64 = 0x3683038;
    /// "OctreeSystem", the scope's name.
    const NAME_RVA: u64 = 0x3700078;
    const SITES: [(RootSite, u64); 2] = [(INIT_SITE, 0x244b8b), (LOAD_SITE, 0x20267d)];

    type Run = extern "C" fn(usize, usize);

    struct Rig {
        _page: Page,
        resolved: ResolvedProfile,
        runs: Vec<(RootSite, Run)>,
    }

    fn rig(exe: &Exe) -> Rig {
        let mut page = Page::new();
        assert_eq!(
            exe.bytes(SCOPE_RVA, 3),
            [0xC2, 0x00, 0x00],
            "the scope is `ret 0`"
        );
        assert_eq!(exe.bytes(HALF_RVA, 4), 32_768.0f32.to_le_bytes());
        let scope = page.code(&[0xC2, 0x00, 0x00]);
        let sign = page.data(exe.bytes(SIGN_RVA, 16));
        let half = page.data(exe.bytes(HALF_RVA, 4));
        let name = page.data(exe.bytes(NAME_RVA, 13));
        let (resize, _) = page.relocate(exe, RESIZE_RVA, RESIZE_LEN, &|target| match target {
            SCOPE_RVA => Some(scope),
            SIGN_RVA => Some(sign),
            NAME_RVA => Some(name),
            _ => None,
        });
        let mut targets = vec![ResolvedTarget {
            name: RESIZE.into(),
            address: resize as u64,
            image_index: 0,
            required: false,
        }];
        let mut runs = Vec::new();
        for (site, rva) in SITES {
            // push rbp; push rsi; sub rsp,0x28; mov rbp,rcx; mov rsi,rdx;
            // <the site's block, its call and the next instruction>;
            // add rsp,0x28; pop rsi; pop rbp; ret
            let entry = page.code(&[
                0x55, 0x56, 0x48, 0x83, 0xEC, 0x28, 0x48, 0x89, 0xCD, 0x48, 0x89, 0xD6,
            ]);
            let len = ROOT_NEXT_OFFSET + site.next.len();
            let (block, offsets) = page.relocate(exe, rva, len, &|target| match target {
                HALF_RVA => Some(half),
                RESIZE_RVA => Some(resize),
                _ => None,
            });
            assert!(offsets.iter().all(|(old, new)| old == new), "same lengths");
            page.code(&[0x48, 0x83, 0xC4, 0x28, 0x5E, 0x5D, 0xC3]);
            targets.push(ResolvedTarget {
                name: site.target.into(),
                address: block as u64,
                image_index: 0,
                required: false,
            });
            // SAFETY: the wrapper above, a function of two pointer arguments.
            runs.push((site, unsafe { std::mem::transmute::<usize, Run>(entry) }));
        }
        Rig {
            _page: page,
            resolved: ResolvedProfile {
                name: "relocated".into(),
                targets,
                absent_optional: Vec::new(),
            },
            runs,
        }
    }

    /// Runs one site on a world of `x` by `y` tiles; the octree's depth
    /// and `+half` (x) as `Resize` left them.
    fn root(run: Run, site: &RootSite, x: i32, y: i32) -> (i32, f32, f32) {
        let mut tiles = [0i32; 16];
        tiles[0] = x;
        tiles[1] = y;
        let mut octree = [0u8; 0x40];
        let mut owner = [0usize; 8];
        owner[OCTREE_FIELD / 8] = octree.as_mut_ptr() as usize;
        let (tiles, owner) = (tiles.as_ptr() as usize, owner.as_ptr() as usize);
        match (site.tiles, site.owner) {
            (Reg::Rbp, Reg::Rsi) => run(tiles, owner),
            (Reg::Rsi, Reg::Rbp) => run(owner, tiles),
            other => panic!("{other:?}"),
        }
        let field = |at: usize| octree[at..at + 4].try_into().unwrap();
        (
            i32::from_le_bytes(field(crate::build_data::native::bigmap::OCTREE_DEPTH_FIELD)),
            f32::from_le_bytes(field(crate::build_data::native::bigmap::OCTREE_HALF_FIELD)),
            f32::from_le_bytes(field(0x10)),
        )
    }

    #[test]
    fn the_games_sites_get_depth_11_only_past_256_tiles() {
        let Some(exe) = Exe::load() else { return };
        let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
        reset();
        let rig = rig(&exe);
        // Stock: the game's own code, unpatched.
        for (site, run) in &rig.runs {
            assert_eq!(root(*run, site, 300, 60), (10, 32_768.0, -32_768.0));
        }
        let splices = install_splices(&rig.resolved, Setting::Depth11).unwrap();
        assert_eq!(splices.len(), 2);
        for (site, run) in &rig.runs {
            for (x, y) in [(130, 130), (250, 50), (256, 2), (2, 256)] {
                assert_eq!(
                    root(*run, site, x, y),
                    (10, 32_768.0, -32_768.0),
                    "{x} x {y}"
                );
            }
            for (x, y) in [(258, 2), (60, 300), (512, 512)] {
                assert_eq!(
                    root(*run, site, x, y),
                    (11, 65_536.0, -65_536.0),
                    "{x} x {y}"
                );
            }
            // Past depth 11: the game's own root stays.
            assert_eq!(root(*run, site, 1000, 186), (10, 32_768.0, -32_768.0));
        }
        for splice in splices {
            // SAFETY: nothing runs the relocated sites now.
            unsafe { splice.detach() }.unwrap();
        }
        // Detached: the game's own code again.
        for (site, run) in &rig.runs {
            assert_eq!(root(*run, site, 300, 60), (10, 32_768.0, -32_768.0));
        }
        reset();
    }

    #[test]
    fn set_to_12_the_games_sites_get_depth_12_past_512_tiles() {
        let Some(exe) = Exe::load() else { return };
        let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
        reset();
        let rig = rig(&exe);
        let splices = install_splices(&rig.resolved, Setting::Depth12).unwrap();
        for (site, run) in &rig.runs {
            assert_eq!(root(*run, site, 176, 176), (10, 32_768.0, -32_768.0));
            assert_eq!(root(*run, site, 60, 300), (11, 65_536.0, -65_536.0));
            assert_eq!(root(*run, site, 512, 62), (11, 65_536.0, -65_536.0));
            assert_eq!(root(*run, site, 1000, 186), (12, 131_072.0, -131_072.0));
            assert_eq!(root(*run, site, 2, 1024), (12, 131_072.0, -131_072.0));
            assert_eq!(root(*run, site, 2, 1026), (10, 32_768.0, -32_768.0));
        }
        for splice in splices {
            // SAFETY: nothing runs the relocated sites now.
            unsafe { splice.detach() }.unwrap();
        }
        reset();
    }

    #[test]
    fn a_site_that_differs_installs_nothing() {
        let Some(exe) = Exe::load() else { return };
        let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
        reset();
        let rig = rig(&exe);
        let mut resolved = rig.resolved.clone();
        // The load site pointed one byte off: neither site is spliced.
        resolved.targets[2].address += 1;
        let error = install_splices(&resolved, Setting::Depth11).err().unwrap();
        assert!(error.contains(ROOT_LOAD_NAME), "{error}");
        for (site, run) in &rig.runs {
            assert_eq!(root(*run, site, 300, 60), (10, 32_768.0, -32_768.0));
        }
        assert!(install_splices(&rig.resolved, Setting::Off).is_err());
        reset();
    }

    const ROOT_LOAD_NAME: &str = crate::build_data::native::bigmap::ROOT_LOAD;
}
