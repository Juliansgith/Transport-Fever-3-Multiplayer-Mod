//! The memory gate's constants for TF3 build 40408: the one place to
//! recalibrate how much memory the New Game page expects a size to need.
//!
//! **MEASURED, Stage 0, 2026-10-05** (docs/BIGMAPS.md, "Stage 0"): stock
//! TF3 build 40408 on Windows 11 (94 GB of memory), no Big Maps code, the
//! game's own New Game at Gigantomaniac, temperate, one run each of 1:1
//! (112 x 112, 822.084 km²) and 1:5 (50 x 250, 819.2 km²). The sources are
//! the game log (`stdout.txt`) and a logger reading the game's private
//! bytes every 5 seconds.
//!
//! - [`GENERATION_MB_PER_KM2`] is the terrain toolkit's MB per km², the
//!   worse of the two runs: 1:1 logged "Terrain toolkit used 41 maps and
//!   8428 MB" (10.25 MB/km²), 1:5 "38 maps and 7785 MB" (9.50).
//! - [`GAME_BASE_MB`] is the 1:1 run's peak private bytes during
//!   generation, 14,210 MB, less that run's toolkit, 8,428 MB: 5,782 MB,
//!   rounded up to 5,800. The 1:5 run's peak is not clean (the previous
//!   world, about 16 GB, was still resident when it started), so 1:1 is
//!   the only peak.
//!
//! The sample is small: one climate, one run of each shape, on Windows.
//! **Not measured yet:**
//!
//! - other climates. A subarctic log of 2026-09-30 showed "49 maps and
//!   10074 MB" for a Gigantomaniac-sized map, 12.25 MB/km², so another
//!   climate may need more than this gate charges;
//! - Linux and Proton;
//! - sizes bigger than stock: the law is a straight line through stock
//!   Gigantomaniac only, and the added rows (128 to 176 tiles) are
//!   extrapolated from it;
//! - the save's size on disk.
//!
//! The gate has no margin of its own: the page offers a row when its
//! expected peak, `area_km2 * GENERATION_MB_PER_KM2 + GAME_BASE_MB`
//! ([`crate::ceilings`]), is at most the machine's physical memory. To
//! recalibrate from new runs:
//!
//! 1. change the two constants here (and their sources in
//!    [`crate::world::WorldModel::TF3_BUILD_40408`]);
//! 2. regenerate the mod's ladder, whose `peakMb` values come from them:
//!    `cargo run -p tpf3mp-bigmap -- --config
//!    crates/tpf3mp-bigmap/tpf3mp_bigmap.stage1.toml lua >
//!    mod/tpf3mp_bigmap_1/content/scripts/tpf3mp_bigmap/ladder.lua`
//!    (a test fails while the two disagree);
//! 3. update the Stage 0 table, the "expected peak" column and the law in
//!    docs/BIGMAPS.md.

/// MEASURED (Stage 0, 2026-10-05, build 40408, Windows, temperate, one run
/// per shape): generation's terrain toolkit, in MB per km² of map. The
/// worse of 1:1's 10.25 and 1:5's 9.50 (see the module docs).
pub const GENERATION_MB_PER_KM2: f64 = 10.25;

/// MEASURED (Stage 0, 2026-10-05, build 40408, Windows, temperate, one
/// 1:1 run): the game's own memory at generation's peak, on top of the
/// toolkit, in MB. 14,210 MB peak private bytes less 8,428 MB of toolkit,
/// rounded up from 5,782.
pub const GAME_BASE_MB: u32 = 5800;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{Evidence, WorldModel};

    #[test]
    fn the_tf3_world_charges_the_gates_constants_and_says_they_are_measured() {
        const TF3: WorldModel = WorldModel::TF3_BUILD_40408;
        assert_eq!(TF3.generation_mb_per_km2.value, GENERATION_MB_PER_KM2);
        assert_eq!(TF3.game_base_mb.value, GAME_BASE_MB);
        // Stage 0 measured both.
        assert_eq!(TF3.generation_mb_per_km2.evidence, Evidence::Measured);
        assert_eq!(TF3.game_base_mb.evidence, Evidence::Measured);
    }

    #[test]
    fn the_constants_reproduce_the_stage_0_runs() {
        const TF3: WorldModel = WorldModel::TF3_BUILD_40408;
        // 1:1, 112 x 112: "Terrain toolkit used 41 maps and 8428 MB".
        let toolkit = TF3.area_km2(112, 112) * GENERATION_MB_PER_KM2;
        assert!((toolkit - 8_428.0).abs() < 5.0, "{toolkit}");
        // Its peak private bytes during generation, 14,210 MB, is covered.
        let peak = toolkit + f64::from(GAME_BASE_MB);
        assert!((14_210.0..14_250.0).contains(&peak), "{peak}");
        // 1:5, 50 x 250, logged 7,785 MB: charged no less.
        assert!(TF3.area_km2(50, 250) * GENERATION_MB_PER_KM2 >= 7_785.0);
    }
}
