//! The GUI's native views see the player's company in a room (docs/HOOKS.md,
//! "The views' player"; investigation/TF3_LOCAL_PLAYER_2026-10-01.md, "The
//! map's markers and overlays").
//!
//! The station icons above the map, the line and catchment overlays and
//! their colours, what the selector lets the player pick, and two React
//! components decide "the player's own" natively: each asks the GUI's
//! `IGameStateProvider` for the `GameState` and reads its player
//! (`+0x20c`) inline, with no helper in between. That `GameState` is one of
//! the simulation's two buffers (the probe, seen 2026-10-02), so its player
//! is never written. Instead each of those reads is spliced
//! (`tpf3mp_hookcore::detour::Splice`) right after it, and in a room the
//! register that holds the save's player is given the player's company,
//! the one the GUI notes (`note("tpf3mp.company")`). Two reads compare the
//! player with an owner straight away (`cmp [reg], eax`); there the splice
//! is on the read and the compare, and the owner's pointer is pointed at a
//! copy that answers as the company would: the save's player where the
//! owner is the company, and no one where it is the save's player.
//!
//! Every site is a UI function (listed in the profile with its callers:
//! `UI::HudIconManager`, `UI::StationViewer`, the selector, `UI::ViewCreator`,
//! `UI::layers::CatchmentAreaHelper`, `UI::layers::LayerManager`'s colours,
//! two `UI::react` components), reached from the GUI's frame, never from
//! `GameSim::Step`; what it computes only draws or picks. Each site's bytes
//! are checked before it is spliced and a site that differs is left alone.
//! Outside a room, for the room's first company, while either note is
//! missing, or with [`ENV`]`=0`, every read answers as the game's.

#![allow(unsafe_code)]

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicU64, Ordering};

use tpf3mp_hookcore::detour::{SavedRegs, Splice};
use tpf3mp_hookcore::profile::ResolvedProfile;

/// The kill switch: `0` (or `off`, `false`, `no`) leaves every view the
/// save's player's.
pub const ENV: &str = "TPF3MP_HOOK_GUI_COMPANY";
/// The name in hook.log.
pub const FIX: &str = "view-company";

/// Which register a site's player is in, and how the site uses it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reg {
    Rax,
    Rbx,
    Rdx,
    R8,
    R14,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The register holds the player read just before the splice.
    Value,
    /// The register points at an owner the next instruction compares with
    /// the player.
    Owner,
}

/// One spliced read.
#[derive(Debug, Clone, Copy)]
pub struct Site {
    /// The profile target (at the splice).
    pub name: &'static str,
    /// The bytes the splice takes (whole instructions).
    pub expected: &'static [u8],
    pub reg: Reg,
    pub kind: Kind,
    /// What it decides, for hook.log.
    pub what: &'static str,
}

pub const SITES: [Site; 16] = [
    Site {
        name: "view: HudIconManager::PreemptiveOctreeTraversal/player",
        expected: &[0x4C, 0x89, 0x75, 0xB8, 0x48, 0x89, 0x5D, 0xC0],
        reg: Reg::Rax,
        kind: Kind::Value,
        what: "the icons above the map's stations",
    },
    Site {
        name: "view: StationViewer::vf4/player",
        expected: &[0x48, 0x8B, 0x46, 0x10, 0x4C, 0x8D, 0x70, 0x78],
        reg: Reg::Rbx,
        kind: Kind::Value,
        what: "the station viewer",
    },
    Site {
        name: "view: CSelector pick/player",
        expected: &[0x48, 0x8B, 0x47, 0x10, 0x4C, 0x8D, 0x70, 0x78],
        reg: Reg::Rbx,
        kind: Kind::Value,
        what: "what the selector picks",
    },
    Site {
        name: "view: ViewCreator::vf1/player",
        expected: &[0x48, 0x8D, 0x55, 0x38, 0x48, 0x89, 0x45, 0x38],
        reg: Reg::Rbx,
        kind: Kind::Value,
        what: "the selector's filter",
    },
    Site {
        name: "view: CatchmentAreaHelper/player 1",
        expected: &[0x48, 0x8B, 0x5D, 0x90, 0x48, 0x8B, 0x53, 0x10],
        reg: Reg::R8,
        kind: Kind::Value,
        what: "the catchment overlay",
    },
    Site {
        name: "view: CatchmentAreaHelper/player 2",
        expected: &[0x40, 0x88, 0x7C, 0x24, 0x20],
        reg: Reg::Rbx,
        kind: Kind::Value,
        what: "the catchment overlay",
    },
    Site {
        name: "view: CatchmentAreaHelper/player 3",
        expected: &[0x4D, 0x8D, 0x77, 0x10, 0x49, 0x8B, 0xCE],
        reg: Reg::Rbx,
        kind: Kind::Value,
        what: "the catchment overlay",
    },
    Site {
        name: "view: CatchmentAreaHelper/player 4",
        expected: &[0xC6, 0x44, 0x24, 0x20, 0x00],
        reg: Reg::Rbx,
        kind: Kind::Value,
        what: "the catchment overlay",
    },
    Site {
        name: "view: CatchmentAreaHelper/owner test",
        expected: &[0x8B, 0x80, 0x0C, 0x02, 0x00, 0x00, 0x39, 0x03],
        reg: Reg::Rbx,
        kind: Kind::Owner,
        what: "the catchment overlay's own stations",
    },
    Site {
        name: "view: LayerManagerColorMap/player 1",
        expected: &[0x49, 0x8B, 0x10, 0x49, 0x8B, 0x48, 0x10],
        reg: Reg::Rax,
        kind: Kind::Value,
        what: "the map layers' colours",
    },
    Site {
        name: "view: LayerManagerColorMap/player 2",
        expected: &[0x48, 0x8B, 0x79, 0x38, 0x48, 0x8B, 0x71, 0x30],
        reg: Reg::Rbx,
        kind: Kind::Value,
        what: "the map layers' colours",
    },
    Site {
        name: "view: LayerManagerColorMap/player 3",
        expected: &[0x48, 0x8B, 0x79, 0x38, 0x48, 0x8B, 0x71, 0x30],
        reg: Reg::Rbx,
        kind: Kind::Value,
        what: "the map layers' colours",
    },
    Site {
        name: "view: LayerManager colour lambda/player",
        expected: &[0x48, 0x8B, 0x51, 0x10, 0x4C, 0x8B, 0x09],
        reg: Reg::Rax,
        kind: Kind::Value,
        what: "the map layers' colours",
    },
    Site {
        name: "view: LayerManager colour/owner test",
        expected: &[0x8B, 0x85, 0x0C, 0x02, 0x00, 0x00, 0x39, 0x02],
        reg: Reg::Rdx,
        kind: Kind::Owner,
        what: "the map layers' own lines and stations",
    },
    Site {
        name: "view: react RendererComponentDelegate/player",
        expected: &[0x49, 0x8B, 0x07, 0x49, 0x8B, 0xCF],
        reg: Reg::Rbx,
        kind: Kind::Value,
        what: "a React renderer component",
    },
    Site {
        name: "view: react RailroadCrossingComp/player",
        expected: &[0x48, 0x8B, 0x03, 0x48, 0x8B, 0x08],
        reg: Reg::R14,
        kind: Kind::Value,
        what: "the railroad crossing component",
    },
];

/// The company and the save's player while a room asks for the company,
/// each -1 otherwise; refreshed from the notes once a frame on the main
/// thread ([`refresh`]), read by the sites on whatever thread they run.
static COMPANY: AtomicI64 = AtomicI64::new(-1);
static SAVE: AtomicI64 = AtomicI64::new(-1);
static ON: AtomicBool = AtomicBool::new(false);
static BROKEN: AtomicBool = AtomicBool::new(false);
/// The owner a pointer site compares when the owner is the company (the
/// save's player, as the read gives), and one no player is.
static SAVE_SLOT: AtomicI32 = AtomicI32::new(-1);
static NONE_SLOT: AtomicI32 = AtomicI32::new(-2);
/// Per site, how many reads it answered with the company; the first is said.
static ANSWERED: [AtomicU64; 16] = [const { AtomicU64::new(0) }; 16];

fn noted(key: &str) -> Option<i64> {
    crate::lua::noted(key)
        .and_then(|v| v.trim().parse::<i64>().ok())
        .filter(|&v| (0..=i64::from(i32::MAX)).contains(&v))
}

/// The company to answer with and the save's player, when the views are to
/// see the company: in a room, both noted, and the company not the save's.
pub fn wanted(room: bool, save: Option<i64>, company: Option<i64>) -> Option<(i64, i64)> {
    match (room, save, company) {
        (true, Some(save), Some(company)) if company != save => Some((company, save)),
        _ => None,
    }
}

/// Once a frame, on the main thread: what the sites answer until the next.
pub fn refresh() {
    if !ON.load(Ordering::Acquire) {
        return;
    }
    let pair = wanted(
        crate::lua::in_room(),
        noted("tpf3mp.player"),
        noted(crate::toolplayer::COMPANY_NOTE),
    );
    let (company, save) = pair.unwrap_or((-1, -1));
    SAVE.store(save, Ordering::Release);
    SAVE_SLOT.store(i32::try_from(save).unwrap_or(-1), Ordering::Release);
    COMPANY.store(company, Ordering::Release);
}

/// What a value site's register becomes: the company where it holds the
/// save's player and the views are to see the company; else as it is.
pub fn value(current: u64, company: i64, save: i64) -> u64 {
    if company < 0 || save < 0 || (current as u32) != save as u32 {
        return current;
    }
    // A 32-bit load zero-extends; the company does so too.
    u64::from(company as u32)
}

/// Where an owner site's pointer should point: at a copy of the save's
/// player where the owner is the company (so the compare matches), at no
/// one where the owner is the save's player; else where it points.
pub fn owner(owner: i32, company: i64, save: i64) -> Option<bool> {
    if company < 0 || save < 0 {
        return None;
    }
    if i64::from(owner) == company {
        Some(true)
    } else if i64::from(owner) == save {
        Some(false)
    } else {
        None
    }
}

fn reg(regs: &mut SavedRegs, reg: Reg) -> &mut u64 {
    match reg {
        Reg::Rax => &mut regs.rax,
        Reg::Rbx => &mut regs.rbx,
        Reg::Rdx => &mut regs.rdx,
        Reg::R8 => &mut regs.r8,
        Reg::R14 => &mut regs.r14,
    }
}

/// A site's work: never unwinds; a panic switches every site off.
fn at_site(index: usize, regs: *mut SavedRegs) {
    if BROKEN.load(Ordering::Acquire) {
        return;
    }
    let done = std::panic::catch_unwind(|| {
        let company = COMPANY.load(Ordering::Acquire);
        let save = SAVE.load(Ordering::Acquire);
        if company < 0 {
            return false;
        }
        let site = SITES[index];
        // SAFETY: the stub's block, held until the hook returns.
        let regs = unsafe { &mut *regs };
        let slot = reg(regs, site.reg);
        match site.kind {
            Kind::Value => {
                let next = value(*slot, company, save);
                let changed = next != *slot;
                *slot = next;
                changed
            }
            Kind::Owner => {
                let at = usize::try_from(*slot).unwrap_or(0);
                if at == 0 || !at.is_multiple_of(4) || !crate::image::readable(at, 4) {
                    return false;
                }
                // SAFETY: four readable bytes, the owner the game compares
                // next; only read.
                let held = unsafe { std::ptr::read_volatile(at as *const i32) };
                match owner(held, company, save) {
                    Some(true) => {
                        *slot = SAVE_SLOT.as_ptr() as u64;
                        true
                    }
                    Some(false) => {
                        *slot = NONE_SLOT.as_ptr() as u64;
                        true
                    }
                    None => false,
                }
            }
        }
    });
    match done {
        Ok(true) => {
            if ANSWERED[index].fetch_add(1, Ordering::Relaxed) == 0 {
                let site = SITES[index];
                crate::log::line(&format!(
                    "{FIX}: {} sees the player's company {} ({})",
                    site.what,
                    COMPANY.load(Ordering::Relaxed),
                    site.name
                ));
            }
        }
        Ok(false) => {}
        Err(_) => BROKEN.store(true, Ordering::Release),
    }
}

macro_rules! hooks {
    ($($name:ident = $index:expr),* $(,)?) => {
        $(
            unsafe extern "system" fn $name(regs: *mut SavedRegs) {
                at_site($index, regs);
            }
        )*
        const HOOKS: [tpf3mp_hookcore::detour::SpliceHook; 16] = [$($name),*];
    };
}

hooks!(
    h0 = 0,
    h1 = 1,
    h2 = 2,
    h3 = 3,
    h4 = 4,
    h5 = 5,
    h6 = 6,
    h7 = 7,
    h8 = 8,
    h9 = 9,
    h10 = 10,
    h11 = 11,
    h12 = 12,
    h13 = 13,
    h14 = 14,
    h15 = 15,
);

/// Splices every site unless [`ENV`] says no; the lines for hook.log.
pub fn install(resolved: &ResolvedProfile) -> Vec<String> {
    install_with(
        resolved,
        crate::ticks::wanted(std::env::var(ENV).ok().as_deref()),
    )
}

pub fn install_with(resolved: &ResolvedProfile, wanted: bool) -> Vec<String> {
    ON.store(false, Ordering::Release);
    if !wanted {
        return vec![format!(
            "{FIX}: off, {ENV} says so; the map's markers and overlays show the save's player's"
        )];
    }
    let mut lines = Vec::new();
    let mut spliced = 0;
    for (index, site) in SITES.iter().enumerate() {
        let Some(target) = resolved.get(site.name) else {
            lines.push(format!(
                "{FIX}: {} stays the save's player's: the profile lacks {}",
                site.what, site.name
            ));
            continue;
        };
        // SAFETY: a site the profile resolved in this build by a unique
        // signature, spliced while the game starts, before any view runs;
        // Splice::install compares the bytes again and refuses others; only
        // instruction boundaries no branch lands inside (tpfre, noted in
        // the profile); the hook changes one register the code after the
        // site reads as the player, or points at a copy of an owner.
        let installed = unsafe {
            Splice::install(
                target.address as usize as *mut u8,
                site.expected,
                site.expected.len(),
                HOOKS[index],
            )
        };
        match installed {
            Ok(splice) => {
                let _kept = std::mem::ManuallyDrop::new(splice);
                spliced += 1;
            }
            Err(error) => lines.push(format!(
                "{FIX}: {} stays the save's player's: {} at {:#x}: {error}",
                site.what, site.name, target.address
            )),
        }
    }
    ON.store(spliced > 0, Ordering::Release);
    lines.insert(
        0,
        format!(
            "{FIX}: {spliced} of {} of the views' player reads see the player's company in a room ({ENV}=0 turns it off)",
            SITES.len()
        ),
    );
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_room_with_another_company_changes_the_views() {
        assert_eq!(wanted(false, Some(214_443), Some(372_631)), None);
        assert_eq!(wanted(true, None, Some(372_631)), None);
        assert_eq!(wanted(true, Some(214_443), None), None);
        assert_eq!(wanted(true, Some(214_443), Some(214_443)), None);
        assert_eq!(
            wanted(true, Some(214_443), Some(372_631)),
            Some((372_631, 214_443))
        );
    }

    #[test]
    fn a_value_site_answers_the_company_for_the_saves_player_only() {
        // eax loaded with the save's player, the rest of rax cleared.
        assert_eq!(value(214_443, 372_631, 214_443), 372_631);
        // Garbage above a 32-bit load is not there; still the low half.
        assert_eq!(value(0xFFFF_FFFF_0003_45AB, 372_631, 214_443), 372_631);
        assert_eq!(value(5, 372_631, 214_443), 5, "another value is left");
        assert_eq!(value(214_443, -1, 214_443), 214_443, "not in a room");
    }

    #[test]
    fn an_owner_site_answers_as_the_company_would() {
        assert_eq!(owner(372_631, 372_631, 214_443), Some(true));
        assert_eq!(owner(214_443, 372_631, 214_443), Some(false));
        assert_eq!(owner(400_000, 372_631, 214_443), None);
        assert_eq!(owner(372_631, -1, 214_443), None);
    }

    #[test]
    fn every_site_is_whole_and_distinct() {
        for site in SITES {
            assert!((5..=16).contains(&site.expected.len()), "{}", site.name);
        }
        let mut names: Vec<&str> = SITES.iter().map(|s| s.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), SITES.len());
        // The owner sites take the read and the compare whole.
        for site in SITES.iter().filter(|s| s.kind == Kind::Owner) {
            assert_eq!(site.expected[0], 0x8B, "{}: a load", site.name);
            assert_eq!(
                &site.expected[2..6],
                &[0x0C, 0x02, 0x00, 0x00],
                "{}: of +0x20c",
                site.name
            );
            assert_eq!(site.expected[6], 0x39, "{}: then a compare", site.name);
        }
    }

    #[test]
    fn off_unless_wanted() {
        let empty = ResolvedProfile {
            name: String::new(),
            targets: Vec::new(),
            absent_optional: Vec::new(),
        };
        assert!(install_with(&empty, false)[0].contains("off"));
        let lines = install_with(&empty, true);
        assert!(lines[0].starts_with(&format!("{FIX}: 0 of 16")));
        refresh();
        assert_eq!(COMPANY.load(Ordering::Relaxed), -1);
    }
}
