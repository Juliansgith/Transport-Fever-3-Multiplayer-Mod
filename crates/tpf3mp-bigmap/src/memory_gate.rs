//! The memory gate's constants for TF3 build 40408: the one place to
//! recalibrate how much memory the New Game page expects a size to need.
//!
//! **PROVISIONAL, pending the Stage 0 measurements** (docs/BIGMAPS.md,
//! "Stage 0"). Neither number has been measured as a generation peak in a
//! running TF3:
//!
//! - [`GENERATION_MB_PER_KM2`] is derived from a single game log's
//!   "Terrain toolkit used 49 maps and 10074 MB" line for one 56 x 224
//!   subarctic map, read as all of the toolkit's maps alive at once, which
//!   is how TPF2 behaved and is not yet checked on TF3;
//! - [`GAME_BASE_MB`] is TPF2's measured own use, carried over to TF3
//!   without any TF3 evidence.
//!
//! The expected peak of a size is `area_km2 * GENERATION_MB_PER_KM2 +
//! GAME_BASE_MB` ([`crate::ceilings`]); the page offers a row only when that
//! fits the machine's physical memory. To recalibrate once Stage 0's table
//! is filled in:
//!
//! 1. change the two constants here (and their evidence in
//!    [`crate::world::WorldModel::TF3_BUILD_40408`] from derived/assumed to
//!    measured, with the runs as the source);
//! 2. regenerate the mod's ladder, whose `peakMb` values come from them:
//!    `cargo run -p tpf3mp-bigmap -- --config
//!    crates/tpf3mp-bigmap/tpf3mp_bigmap.stage1.toml lua >
//!    mod/tpf3mp_bigmap_1/content/scripts/tpf3mp_bigmap/ladder.lua`
//!    (a test fails while the two disagree);
//! 3. update the "expected peak" column and the law in docs/BIGMAPS.md.

/// PROVISIONAL: generation's peak memory per km² of map, in MB. Derived
/// from one TF3 log, not measured as a peak (see the module docs).
pub const GENERATION_MB_PER_KM2: f64 = 12.25;

/// PROVISIONAL: the game's own memory use on top of generation, in MB.
/// TPF2's figure (about 4 GB), assumed for TF3 without evidence.
pub const GAME_BASE_MB: u32 = 4096;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{Evidence, WorldModel};

    #[test]
    fn the_tf3_world_charges_the_gates_constants_and_says_they_are_unmeasured() {
        const TF3: WorldModel = WorldModel::TF3_BUILD_40408;
        assert_eq!(TF3.generation_mb_per_km2.value, GENERATION_MB_PER_KM2);
        assert_eq!(TF3.game_base_mb.value, GAME_BASE_MB);
        // Until Stage 0 replaces them, neither may claim to be measured.
        assert_ne!(TF3.generation_mb_per_km2.evidence, Evidence::Measured);
        assert_ne!(TF3.game_base_mb.evidence, Evidence::Measured);
    }
}
