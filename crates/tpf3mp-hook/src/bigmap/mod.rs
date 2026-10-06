//! Big maps' native patches for TF3 (docs/BIGMAPS.md;
//! investigation/TF3_BIGMAPS_256KM_2026-10-05.md).
//!
//! Each patch is **opt-in**, set in the game's environment and off by
//! default, and each is **derived from the world**: it changes nothing
//! unless the world being allocated needs it, so a stock-sized or Stage 1
//! world runs the game's own code whether a game has the patch or not.
//! Each **fails closed**: a target the profile did not resolve, or a site
//! whose bytes are not what the patch was read on, leaves the game stock
//! and says why in `hook.log`.
//!
//! What the patches change is not in the save. Every game of a room on a
//! world that needs one must run it, with the same setting; the room does
//! not check that yet (docs/BIGMAPS.md, "Stage 2").
//!
//! - [`octree`]: the octree root at depth 11 (±65,536 m, 512 tiles) for a
//!   world with an axis over 256 tiles, [`OCTREE_ENV`].

pub mod octree;

#[cfg(all(test, windows, target_arch = "x86_64"))]
pub(crate) mod original;

use tpf3mp_hookcore::profile::ResolvedProfile;

pub use octree::OCTREE_ENV;

/// Whether a switch's value turns it on: `1`, `on`, `true` or `yes`. Unset
/// or empty is off, as is anything else, and the caller says so.
pub fn switch(value: Option<&str>) -> Result<bool, String> {
    match value.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
        None | Some("" | "0" | "off" | "false" | "no") => Ok(false),
        Some("1" | "on" | "true" | "yes") => Ok(true),
        Some(other) => Err(format!("{other:?} is not 1 or 0")),
    }
}

/// Installs every big-map patch its setting asks for, from the targets at
/// their addresses in this process. Returns the lines for `hook.log`.
pub fn install(resolved: &ResolvedProfile) -> Vec<String> {
    vec![octree::install(resolved)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_switch_is_off_unless_it_says_on() {
        for off in [
            None,
            Some(""),
            Some("0"),
            Some(" OFF "),
            Some("false"),
            Some("no"),
        ] {
            assert_eq!(switch(off), Ok(false), "{off:?}");
        }
        for on in ["1", "on", " TRUE ", "yes"] {
            assert_eq!(switch(Some(on)), Ok(true), "{on}");
        }
        assert!(switch(Some("2")).is_err());
    }
}
