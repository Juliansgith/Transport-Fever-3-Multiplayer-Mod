//! What every player in a room must share for a big map.
//!
//! A big map is not only a bigger save. The settings that let the engine
//! build it change how it runs: the octree's root decides which entities a
//! lookup finds, the street raster's cell decides where streets may go, and
//! town and industry placement, which runs again whenever an industry is
//! founded, reads the placement budget. Every game in the room must run with
//! the same values, or they diverge (Big Maps: "Every peer needs the same
//! `octree_depth`"). The room compares one fingerprint of them; the agent
//! refuses to join with another, and names what differs.
//!
//! What only changes the menu or the screen (the added rows, the minimap)
//! is not part of the terms: a player may have it or not.

use sha2::{Digest, Sha256};

use crate::ceilings::SizeReport;
use crate::config::Config;

/// The big-map settings a room's games must share.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Terms {
    /// The world's size, in tiles.
    pub tiles: (u32, u32),
    /// The octree depth the world runs at.
    pub octree_depth: u8,
    /// The street raster's cell, in metres.
    pub street_cell_m: u32,
    /// Placement attempts per worker batch.
    pub placement_attempts: u32,
    /// Whether the added town and industry levels are in use.
    pub density_levels: bool,
    /// The emission throttle's N (1: the game's own). A proposal that
    /// changes the simulation (docs/BIGMAPS.md, "Simulation switches
    /// (proposal)").
    pub emission_every: u32,
}

impl Terms {
    /// The terms of a world checked as `report` under `config`.
    pub fn of(report: &SizeReport, config: &Config) -> Self {
        Self {
            tiles: report.tiles,
            octree_depth: report.octree_depth,
            street_cell_m: report.street_cell_m,
            placement_attempts: config.generation.placement_attempts,
            density_levels: config.generation.density_levels,
            emission_every: config.simulation.emission_every,
        }
    }

    /// A fingerprint of the terms, the same on every platform: SHA-256 of a
    /// fixed text form, versioned so a future field changes every print.
    /// The simulation switches add a line only when they are not the
    /// game's own, so a room that does not use them keeps its print.
    pub fn fingerprint(&self) -> [u8; 32] {
        let mut text = format!(
            "tpf3mp-bigmap-terms/1\ntiles={}x{}\noctree_depth={}\nstreet_cell_m={}\nplacement_attempts={}\ndensity_levels={}\n",
            self.tiles.0,
            self.tiles.1,
            self.octree_depth,
            self.street_cell_m,
            self.placement_attempts,
            u8::from(self.density_levels),
        );
        if self.emission_every != 1 {
            text.push_str(&format!("emission_every={}\n", self.emission_every));
        }
        Sha256::digest(text.as_bytes()).into()
    }

    /// What differs between these terms and the room's, by name, for the
    /// message that tells a player what to change.
    pub fn differences(&self, room: &Self) -> Vec<&'static str> {
        let mut differ = Vec::new();
        if self.tiles != room.tiles {
            differ.push("map size");
        }
        if self.octree_depth != room.octree_depth {
            differ.push("octree depth");
        }
        if self.street_cell_m != room.street_cell_m {
            differ.push("street raster");
        }
        if self.placement_attempts != room.placement_attempts {
            differ.push("placement attempts");
        }
        if self.density_levels != room.density_levels {
            differ.push("density levels");
        }
        if self.emission_every != room.emission_every {
            differ.push("emission throttle");
        }
        differ
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ceilings::check;
    use crate::world::WorldModel;

    fn terms(text: &str, tiles: u32) -> Terms {
        let config = Config::from_toml(text).unwrap().0;
        Terms::of(
            &check(&WorldModel::TPF2_BUILD_35924, &config, tiles, tiles),
            &config,
        )
    }

    #[test]
    fn the_same_settings_give_the_same_print() {
        let a = terms("[limits]\nstreet_raster = true\noctree_depth = 11", 320);
        let b = terms(
            "[limits]\nstreet_raster = true\noctree_depth = 11\n[ui]\nminimap = true",
            320,
        );
        assert_eq!(a, b, "the minimap is not a term");
        assert_eq!(a.fingerprint(), b.fingerprint());
        assert!(a.differences(&b).is_empty());
    }

    #[test]
    fn a_different_setting_is_named() {
        let room = terms("[limits]\nstreet_raster = true\noctree_depth = 11", 320);
        let mine = terms("[limits]\nstreet_raster = true\noctree_depth = 12", 320);
        assert_ne!(room.fingerprint(), mine.fingerprint());
        assert_eq!(mine.differences(&room), ["octree depth"]);
        let fewer = terms(
            "[limits]\nstreet_raster = true\noctree_depth = 11\n[generation]\nplacement_attempts = 50",
            320,
        );
        assert_eq!(fewer.differences(&room), ["placement attempts"]);
    }

    #[test]
    fn the_emission_throttle_is_a_term() {
        let room = terms("[simulation]\nemission_every = 4", 320);
        let stock = terms("", 320);
        assert_ne!(room.fingerprint(), stock.fingerprint());
        assert_eq!(stock.differences(&room), ["emission throttle"]);
        let other = terms("[simulation]\nemission_every = 2", 320);
        assert_eq!(other.differences(&room), ["emission throttle"]);
        assert_eq!(
            terms("[simulation]\nemission_every = 4", 320).fingerprint(),
            room.fingerprint()
        );
    }

    #[test]
    fn the_print_is_pinned() {
        // Changing the text form changes every room's print: bump its version.
        let stock = terms("", 96);
        assert_eq!(stock.octree_depth, 9);
        let hex: String = stock
            .fingerprint()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let text = "tpf3mp-bigmap-terms/1\ntiles=96x96\noctree_depth=9\nstreet_cell_m=1\nplacement_attempts=200\ndensity_levels=0\n";
        let expected: String = Sha256::digest(text.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(hex, expected);
    }
}
