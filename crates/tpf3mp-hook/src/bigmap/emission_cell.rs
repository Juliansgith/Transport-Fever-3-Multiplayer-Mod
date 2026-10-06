//! Coarser emission cells for new worlds: a PROPOSAL that CHANGES
//! SIMULATION RESULTS.
//!
//! `ecs::component::CreateEmissionGrid` gives a new world's noise and
//! pollution grids 16 cells per tile a side: 16 m cells on 256 m tiles,
//! 25.6 M cells per grid on 100 x 1000 tiles, five buffers of 102 MB
//! (investigation/TF3_BIGMAP_SIM_COST_2026-10-05.md §1.3). With
//! [`CELL_ENV`] set to `32`, its four `shl reg,4` become `shl reg,3` and its
//! cell factor load reads the game's own 0.125f instead of 0.0625f: 8 cells
//! per tile, 32 m cells, a quarter of the cells, memory and sweep time.
//!
//! **Every consumer reads the geometry from the grid** (CONFIRMED-static,
//! the investigation's "Switches built"): the emitters' splat and its
//! sub-grid split, `Diffuse`, `Wind` (its wind speed cap is 4 m either
//! way), `Average`, `GetSumPolygonEmission`, `GetInterpolatedEmission` (the
//! script getters, animals' scores), the visualisation grids and the save:
//! the loader reads `X0`, `Y0`, size and `gridPointSize` from the save and
//! never compares them with the map. So nothing asserts or misreads, and
//! a world's cell size travels with its save: a coarse world loads coarse
//! in any game, a stock save loads stock with the switch on. There is no
//! conflict for a load to refuse; the switch acts only when a world is
//! generated (`CreateEmissionGrid` runs only on the new-game path).
//!
//! **What changes**: the diffusion, decay and averaging weights are per
//! cell and step, so emissions spread twice as far in metres; a point
//! emitter deposits the same amount per (4x larger) cell, a radius emitter
//! a quarter; a town's sum covers a quarter of the cells. Town noise and
//! pollution, ratings, eco levels and animal scores shift.
//!
//! **Rooms**: a room's world reaches every game as the owner's save, so
//! only the game that generates it needs the switch, and every game then
//! runs the same grid. The hook says the setting in `hook.log`.
//!
//! **Fails closed**: the body between the first shift and the factor's
//! multiplies is checked byte for byte, the factor load must read 0.0625f
//! and the coarse constant must hold 0.125f; all five sites are rewritten
//! or none.

#![allow(unsafe_code)]

use tpf3mp_hookcore::detour::Rewrite;
use tpf3mp_hookcore::profile::ResolvedProfile;

pub use crate::build_data::native::simswitch::{
    CREATE_GRID, CREATE_GRID_BODY, CREATE_GRID_BODY_OFFSET, CREATE_GRID_COARSE_FACTOR_DELTA,
    CREATE_GRID_FACTOR_LOAD, CREATE_GRID_SHIFTS, VMOVSS_XMM2_RIP,
};

/// The patch's name in `hook.log`.
pub const FIX: &str = "big maps: emission cells (PROPOSAL, changes simulation results)";

/// `32` in the game's environment gives new worlds 8 emission cells per
/// tile (32 m on 256 m tiles); unset, `0` or `16` keeps the game's 16.
pub const CELL_ENV: &str = "TPF3MP_BIGMAP_EMISSION_CELL";

/// The setting from [`CELL_ENV`]'s value: whether to coarsen.
pub fn cell_setting(value: Option<&str>) -> Result<bool, String> {
    match value.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
        None | Some("" | "0" | "16" | "off" | "no") => Ok(false),
        Some("32") => Ok(true),
        Some(other) => Err(format!(
            "{CELL_ENV}={other:?} is not 32 (or 16 for the game's own)"
        )),
    }
}

/// One rewrite, at an offset from the function: the bytes found and the
/// bytes written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub offset: usize,
    pub expected: Vec<u8>,
    pub replacement: Vec<u8>,
}

/// The rewrites for the function whose bytes from its start are `code`
/// (at least to the end of the checked body), at `function`; `read_f32`
/// reads a constant at an address. Checks everything first.
pub fn edits(
    code: &[u8],
    function: usize,
    read_f32: &dyn Fn(usize) -> Option<f32>,
) -> Result<Vec<Edit>, String> {
    let body = code
        .get(CREATE_GRID_BODY_OFFSET..CREATE_GRID_BODY_OFFSET + CREATE_GRID_BODY.len())
        .ok_or_else(|| format!("{CREATE_GRID}'s body is short"))?;
    let disp_at = CREATE_GRID_FACTOR_LOAD + VMOVSS_XMM2_RIP.len() - CREATE_GRID_BODY_OFFSET;
    for (i, (found, expected)) in body.iter().zip(CREATE_GRID_BODY).enumerate() {
        if (disp_at..disp_at + 4).contains(&i) {
            continue;
        }
        if *found != expected {
            return Err(format!(
                "{CREATE_GRID} differs at +{:#x}; not the code this patch was read on",
                CREATE_GRID_BODY_OFFSET + i
            ));
        }
    }
    let load = &code[CREATE_GRID_FACTOR_LOAD..CREATE_GRID_FACTOR_LOAD + 8];
    let disp = i32::from_le_bytes([load[4], load[5], load[6], load[7]]);
    let next = function + CREATE_GRID_FACTOR_LOAD + 8;
    let stock = next.wrapping_add_signed(disp as isize);
    if read_f32(stock) != Some(0.0625) {
        return Err(format!(
            "{CREATE_GRID}'s cell factor at {stock:#x} is not 0.0625"
        ));
    }
    let coarse = stock + CREATE_GRID_COARSE_FACTOR_DELTA;
    if read_f32(coarse) != Some(0.125) {
        return Err(format!("the constant at {coarse:#x} is not 0.125"));
    }
    let new_disp = i32::try_from(coarse as i64 - next as i64)
        .map_err(|_| "the coarse factor is out of reach".to_owned())?;
    let mut edits: Vec<Edit> = CREATE_GRID_SHIFTS
        .iter()
        .map(|&offset| {
            let expected = code[offset..offset + 3].to_vec();
            let mut replacement = expected.clone();
            replacement[2] = 3;
            Edit {
                offset,
                expected,
                replacement,
            }
        })
        .collect();
    let mut replacement = load.to_vec();
    replacement[4..].copy_from_slice(&new_disp.to_le_bytes());
    edits.push(Edit {
        offset: CREATE_GRID_FACTOR_LOAD,
        expected: load.to_vec(),
        replacement,
    });
    Ok(edits)
}

/// What installing came to, for `hook.log`.
pub fn outcome_line(installed: bool, reason: &str) -> String {
    if installed {
        format!("{FIX}: installed ({reason})")
    } else {
        format!("{FIX}: off, {reason}")
    }
}

/// Installs the patch if [`CELL_ENV`] asks for it. Returns the line for
/// `hook.log`.
pub fn install(resolved: &ResolvedProfile) -> String {
    match cell_setting(std::env::var(CELL_ENV).ok().as_deref()) {
        Ok(wanted) => install_with(resolved, wanted),
        Err(why) => outcome_line(false, &why),
    }
}

pub fn install_with(resolved: &ResolvedProfile, wanted: bool) -> String {
    match install_edits(resolved, wanted) {
        Ok((at, rewrites)) => {
            std::mem::forget(rewrites);
            outcome_line(
                true,
                &format!(
                    "{CELL_ENV}=32 at {at:#x}: worlds generated from now get 8 emission cells per tile (32 m on 256 m tiles) instead of 16; a loaded save keeps the cells it was made with; noise and pollution results differ from the game's"
                ),
            )
        }
        Err(why) => outcome_line(false, &why),
    }
}

/// Reads a float the image holds, if readable.
fn read_f32(address: usize) -> Option<f32> {
    // SAFETY: four bytes, checked readable first.
    crate::image::readable(address, 4)
        .then(|| unsafe { std::ptr::read_unaligned(address as *const f32) })
}

pub(crate) fn install_edits(
    resolved: &ResolvedProfile,
    wanted: bool,
) -> Result<(usize, Vec<Rewrite>), String> {
    if !wanted {
        return Err(format!(
            "{CELL_ENV} is not 32; the game's 16 cells per tile"
        ));
    }
    let at = resolved
        .get(CREATE_GRID)
        .ok_or_else(|| format!("the profile has no {CREATE_GRID:?}"))?
        .address;
    let function = usize::try_from(at).map_err(|_| "an address past usize".to_owned())?;
    let len = CREATE_GRID_BODY_OFFSET + CREATE_GRID_BODY.len();
    if !crate::image::readable(function, len) {
        return Err(format!("{CREATE_GRID} at {function:#x} is unreadable"));
    }
    // SAFETY: `len` readable bytes, checked just above.
    let code = unsafe { std::slice::from_raw_parts(function as *const u8, len) };
    let edits = edits(code, function, &read_f32)?;
    let mut done = Vec::new();
    for edit in &edits {
        // SAFETY: whole instructions in place of whole instructions, each
        // checked above; no world exists yet, so nothing runs them.
        match unsafe {
            Rewrite::install(
                (function + edit.offset) as *mut u8,
                &edit.expected,
                &edit.replacement,
            )
        } {
            Ok(rewrite) => done.push(rewrite),
            Err(error) => {
                for rewrite in done {
                    // SAFETY: as above.
                    let _ = unsafe { rewrite.detach() };
                }
                return Err(format!("{CREATE_GRID} +{:#x}: {error}", edit.offset));
            }
        }
    }
    Ok((function, done))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_setting_is_32_or_the_games_own() {
        for off in [None, Some(""), Some("0"), Some("16"), Some(" OFF ")] {
            assert_eq!(cell_setting(off), Ok(false), "{off:?}");
        }
        assert_eq!(cell_setting(Some(" 32 ")), Ok(true));
        for bad in ["8", "64", "1", "yes"] {
            assert!(cell_setting(Some(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn nothing_installs_when_off_or_without_the_target() {
        let empty = ResolvedProfile {
            name: "empty".into(),
            targets: Vec::new(),
            absent_optional: Vec::new(),
        };
        assert!(install_with(&empty, false).contains(CELL_ENV));
        assert!(install_with(&empty, true).contains("the profile has no"));
    }

    /// A function image: the pinned body at its offset, the factor load
    /// pointed at `stock` (an address, 0.0625 there).
    fn image(function: usize, stock: usize) -> Vec<u8> {
        let mut code = vec![0xCC; CREATE_GRID_BODY_OFFSET + CREATE_GRID_BODY.len()];
        code[CREATE_GRID_BODY_OFFSET..].copy_from_slice(&CREATE_GRID_BODY);
        let next = function + CREATE_GRID_FACTOR_LOAD + 8;
        let disp = i32::try_from(stock as i64 - next as i64).unwrap();
        code[CREATE_GRID_FACTOR_LOAD + 4..CREATE_GRID_FACTOR_LOAD + 8]
            .copy_from_slice(&disp.to_le_bytes());
        code
    }

    #[test]
    fn the_edits_are_the_shifts_and_the_factor_or_nothing() {
        let function = 0x1_40ba_0170;
        let stock = 0x1_436a_3a64;
        let constants = |address: usize| match address {
            a if a == stock => Some(0.0625),
            a if a == stock + CREATE_GRID_COARSE_FACTOR_DELTA => Some(0.125),
            _ => None,
        };
        let code = image(function, stock);
        let edits = edits(&code, function, &constants).unwrap();
        assert_eq!(edits.len(), 5);
        for edit in &edits[..4] {
            assert_eq!(edit.expected[..2], edit.replacement[..2]);
            assert_eq!(
                (edit.expected[2], edit.replacement[2]),
                (4, 3),
                "shl 4 -> 3"
            );
        }
        let load = &edits[4];
        assert_eq!(load.replacement[..4], VMOVSS_XMM2_RIP);
        let disp = i32::from_le_bytes(load.replacement[4..].try_into().unwrap());
        let next = function + CREATE_GRID_FACTOR_LOAD + 8;
        assert_eq!(
            next.wrapping_add_signed(disp as isize),
            stock + CREATE_GRID_COARSE_FACTOR_DELTA
        );
        // Another constant, another byte, another float: nothing.
        let mut moved = code.clone();
        moved[CREATE_GRID_SHIFTS[3] + 2] = 5;
        assert!(edits_err(&moved, function, &constants).contains("differs at +0xb7"));
        let wrong = |address: usize| (address == stock).then_some(0.0625);
        assert!(edits_err(&code, function, &wrong).contains("is not 0.125"));
        let elsewhere = image(function, stock + 8);
        assert!(edits_err(&elsewhere, function, &constants).contains("is not 0.0625"));
    }

    #[cfg(all(windows, target_arch = "x86_64"))]
    #[test]
    fn installed_the_five_sites_change_and_detached_they_are_back() {
        use tpf3mp_hookcore::profile::ResolvedTarget;
        // A function at the start of the buffer, its constants far enough
        // in for the coarse one's distance.
        // Its own region: the rewrites change its protection.
        let len = CREATE_GRID_COARSE_FACTOR_DELTA + 0x2000;
        let page = crate::bigmap::original::Page::with_len(len);
        // SAFETY: a fresh zeroed region of `len` bytes, only this test's.
        let buffer = unsafe { std::slice::from_raw_parts_mut(page.base() as *mut u8, len) };
        let function = buffer.as_ptr() as usize;
        let stock = function + 0x1000;
        let code = image(function, stock);
        buffer[..code.len()].copy_from_slice(&code);
        buffer[0x1000..0x1004].copy_from_slice(&0.0625f32.to_le_bytes());
        let coarse = 0x1000 + CREATE_GRID_COARSE_FACTOR_DELTA;
        buffer[coarse..coarse + 4].copy_from_slice(&0.125f32.to_le_bytes());
        let resolved = ResolvedProfile {
            name: "fake".into(),
            targets: vec![ResolvedTarget {
                name: CREATE_GRID.into(),
                address: function as u64,
                image_index: 0,
                required: false,
            }],
            absent_optional: Vec::new(),
        };
        let (at, rewrites) = install_edits(&resolved, true).unwrap();
        assert_eq!(at, function);
        for &shift in &CREATE_GRID_SHIFTS {
            assert_eq!(buffer[shift + 2], 3);
        }
        assert_ne!(buffer[..code.len()], code[..]);
        for rewrite in rewrites {
            // SAFETY: nothing runs the buffer.
            unsafe { rewrite.detach() }.unwrap();
        }
        assert_eq!(buffer[..code.len()], code[..]);
    }

    fn edits_err(code: &[u8], function: usize, read: &dyn Fn(usize) -> Option<f32>) -> String {
        edits(code, function, read).err().unwrap()
    }
}

/// The patch applied to the game's own bytes, from the executable.
#[cfg(all(test, windows, target_arch = "x86_64"))]
#[allow(clippy::unwrap_used)]
mod original_tests {
    use iced_x86::{Decoder, DecoderOptions, Mnemonic, OpKind, Register};

    use super::*;
    use crate::bigmap::original::Exe;

    const FUNCTION_RVA: u64 = 0xba0170;

    #[test]
    fn the_games_code_becomes_eight_cells_per_tile() {
        let Some(exe) = Exe::load() else { return };
        let len = CREATE_GRID_BODY_OFFSET + CREATE_GRID_BODY.len();
        let mut code = exe.bytes(FUNCTION_RVA, len).to_vec();
        let read = |rva: usize| {
            Some(f32::from_le_bytes(
                exe.bytes(rva as u64, 4).try_into().unwrap(),
            ))
        };
        let edits = edits(&code, FUNCTION_RVA as usize, &read).unwrap();
        for edit in &edits {
            assert_eq!(
                code[edit.offset..edit.offset + edit.expected.len()],
                edit.expected
            );
            code[edit.offset..edit.offset + edit.replacement.len()]
                .copy_from_slice(&edit.replacement);
        }
        let mut decoder = Decoder::with_ip(64, &code, FUNCTION_RVA, DecoderOptions::NONE);
        let mut shifts = Vec::new();
        let mut factor = None;
        for insn in &mut decoder {
            let offset = (insn.ip() - FUNCTION_RVA) as usize;
            if CREATE_GRID_SHIFTS.contains(&offset) {
                assert_eq!(insn.mnemonic(), Mnemonic::Shl);
                assert_eq!(insn.op1_kind(), OpKind::Immediate8);
                shifts.push((insn.op0_register(), insn.immediate8()));
            }
            if offset == CREATE_GRID_FACTOR_LOAD {
                assert_eq!(insn.mnemonic(), Mnemonic::Vmovss);
                assert_eq!(insn.op0_register(), Register::XMM2);
                factor = Some(insn.ip_rel_memory_address());
            }
        }
        assert_eq!(
            shifts,
            [
                (Register::EAX, 3),
                (Register::EAX, 3),
                (Register::EAX, 3),
                (Register::ECX, 3)
            ]
        );
        let factor = factor.unwrap();
        assert_eq!(factor, 0x36b2fa4);
        assert_eq!(read(factor as usize), Some(0.125));
    }
}
