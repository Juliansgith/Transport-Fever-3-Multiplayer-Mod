//! The big-map settings file, `tpf3mp_bigmap.toml`.
//!
//! Every feature ships off, as docs/BIGMAPS.md's rules ask: nothing is on
//! until an in-game measurement on the same map says it helps. Like Big
//! Maps' own config, a size that is odd or too large is clamped with a
//! note rather than refused, since the host picks a size once and the room
//! shares it ([`crate::terms`]). A value the engine depends on everywhere
//! (the octree depth, the placement budget) is refused instead: those are
//! never guessed.
//!
//! ```toml
//! [sizes]
//! add_rows = true        # append the ladder to the size dropdown
//! max_tiles = 512        # the largest edge any row or shape may have
//! # rows = [ { label = "32 x 32 km", tiles = 128 } ]   # else the default ladder
//!
//! [limits]
//! octree_depth = 11      # 0: the game's own choice; 11 to 13: a larger root
//! street_raster = true   # coarsen the street raster's cells past its budget
//! cell_budget_millions = 1500
//!
//! [generation]
//! placement_attempts = 200   # the stock budget; Big Maps offers 50
//! density_levels = false     # town and industry levels for big maps
//!
//! [ui]
//! minimap = false
//! ```

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::ladder::{self, SizeRow};
use crate::world::WorldModel;

/// The largest edge a setting may name, so a typo cannot ask for an absurd
/// map. Big Maps allows the same.
pub const MAX_TILES: u32 = 2048;
/// The octree depths the larger root offers (Big Maps: 11 is 512 tiles, 13
/// is 2048).
pub const OCTREE_DEPTHS: std::ops::RangeInclusive<u8> = 11..=13;
/// The game's own placement budget per worker batch (TPF2).
pub const STOCK_PLACEMENT_ATTEMPTS: u32 = 200;
/// The longest label the dropdown shows whole (Big Maps' rule).
pub const MAX_LABEL: usize = 15;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("the settings file is not valid: {0}")]
    Toml(String),
    #[error("octree_depth must be 0 (the game's own) or 11 to 13, not {0}")]
    OctreeDepth(u8),
    #[error("placement_attempts must be 1 to 200, not {0}")]
    PlacementAttempts(u32),
    #[error("cell_budget_millions must be 1 to 2147, not {0}")]
    CellBudget(u32),
    #[error("the size label {0:?} must be 1 to 15 characters")]
    Label(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub sizes: Sizes,
    pub limits: Limits,
    pub generation: Generation,
    pub ui: Ui,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Sizes {
    pub add_rows: bool,
    pub max_tiles: u32,
    /// The rows to add; the default ladder when absent.
    pub rows: Option<Vec<SizeRow>>,
}

impl Default for Sizes {
    fn default() -> Self {
        Self {
            add_rows: false,
            max_tiles: 512,
            rows: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    /// 0: the depth the game picks; 11 to 13: a larger root.
    pub octree_depth: u8,
    pub street_raster: bool,
    pub cell_budget_millions: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            octree_depth: 0,
            street_raster: false,
            cell_budget_millions: 1500,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Generation {
    pub placement_attempts: u32,
    pub density_levels: bool,
}

impl Default for Generation {
    fn default() -> Self {
        Self {
            placement_attempts: STOCK_PLACEMENT_ATTEMPTS,
            density_levels: false,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Ui {
    pub minimap: bool,
}

impl Config {
    /// The settings in `text`, and a note for each value that was clamped.
    pub fn from_toml(text: &str) -> Result<(Self, Vec<String>), ConfigError> {
        let mut config: Self =
            toml::from_str(text).map_err(|error| ConfigError::Toml(error.to_string()))?;
        let notes = config.check()?;
        Ok((config, notes))
    }

    /// Refuses what must not be guessed, and clamps sizes with a note.
    pub fn check(&mut self) -> Result<Vec<String>, ConfigError> {
        let mut notes = Vec::new();
        let limits = &self.limits;
        if limits.octree_depth != 0 && !OCTREE_DEPTHS.contains(&limits.octree_depth) {
            return Err(ConfigError::OctreeDepth(limits.octree_depth));
        }
        if !(1..=2147).contains(&limits.cell_budget_millions) {
            return Err(ConfigError::CellBudget(limits.cell_budget_millions));
        }
        let attempts = self.generation.placement_attempts;
        if !(1..=STOCK_PLACEMENT_ATTEMPTS).contains(&attempts) {
            return Err(ConfigError::PlacementAttempts(attempts));
        }
        let max = &mut self.sizes.max_tiles;
        let clamped = (*max).clamp(2, MAX_TILES) & !1;
        if clamped != *max {
            notes.push(format!("max_tiles {} is now {clamped}", *max));
            *max = clamped;
        }
        let max = *max;
        for row in self.sizes.rows.iter_mut().flatten() {
            if row.label.is_empty() || row.label.chars().count() > MAX_LABEL {
                return Err(ConfigError::Label(row.label.clone()));
            }
            let clamped = row.tiles.clamp(2, max) & !1;
            if clamped != row.tiles {
                notes.push(format!(
                    "the row {:?}: {} tiles is now {clamped}",
                    row.label, row.tiles
                ));
                row.tiles = clamped;
            }
        }
        Ok(notes)
    }

    /// The rows to add: the configured ones, or the default ladder.
    pub fn rows(&self, world: &WorldModel) -> Vec<SizeRow> {
        match &self.sizes.rows {
            Some(rows) => rows.clone(),
            None => ladder::default_rows(world)
                .into_iter()
                .filter(|row| row.tiles <= self.sizes.max_tiles)
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn everything_ships_off() {
        let (config, notes) = Config::from_toml("").unwrap();
        assert!(notes.is_empty());
        assert!(!config.sizes.add_rows);
        assert_eq!(config.limits.octree_depth, 0);
        assert!(!config.limits.street_raster);
        assert_eq!(config.generation.placement_attempts, 200);
        assert!(!config.generation.density_levels);
        assert!(!config.ui.minimap);
    }

    #[test]
    fn sizes_are_clamped_with_a_note() {
        let (config, notes) = Config::from_toml(
            r#"
            [sizes]
            max_tiles = 513
            rows = [ { label = "odd", tiles = 131 }, { label = "huge", tiles = 9000 } ]
            "#,
        )
        .unwrap();
        assert_eq!(config.sizes.max_tiles, 512);
        let tiles: Vec<u32> = config.sizes.rows.unwrap().iter().map(|r| r.tiles).collect();
        assert_eq!(tiles, [130, 512]);
        assert_eq!(notes.len(), 3);
    }

    #[test]
    fn what_the_engine_depends_on_is_refused_not_guessed() {
        let depth = Config::from_toml("[limits]\noctree_depth = 12").unwrap().0;
        assert_eq!(depth.limits.octree_depth, 12);
        assert_eq!(
            Config::from_toml("[limits]\noctree_depth = 10"),
            Err(ConfigError::OctreeDepth(10))
        );
        assert_eq!(
            Config::from_toml("[generation]\nplacement_attempts = 500"),
            Err(ConfigError::PlacementAttempts(500))
        );
        assert!(matches!(
            Config::from_toml(
                "[sizes]\nrows = [ { label = \"a label that is too long\", tiles = 128 } ]"
            ),
            Err(ConfigError::Label(_))
        ));
        assert!(matches!(
            Config::from_toml("[limits]\noctree = 1"),
            Err(ConfigError::Toml(_))
        ));
    }

    #[test]
    fn the_default_ladder_stops_at_max_tiles() {
        let (config, _) = Config::from_toml("[sizes]\nmax_tiles = 256").unwrap();
        let rows = config.rows(&WorldModel::TPF2_BUILD_35924);
        assert_eq!(rows.last().map(|r| r.tiles), Some(256));
    }
}
