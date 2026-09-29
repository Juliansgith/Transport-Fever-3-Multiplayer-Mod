//! The engine facts a map size depends on.
//!
//! Every number carries its evidence, as docs/BIGMAPS.md asks: measured in a
//! running game or its own log, or derived from decompiled code and not yet
//! seen live. Only TPF2 build 35924 is known. TF3 gets a model of its own
//! once its numbers are measured (docs/BIGMAPS.md, "Measure these first on
//! TPF3"); until then nothing may assume TPF2's hold for it.

/// How a number is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Evidence {
    /// Read from a running game, a process or the game's own log.
    Measured,
    /// Computed from decompiled code; not yet observed live.
    Derived,
}

/// A number and how it is known.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fact<T> {
    pub value: T,
    pub evidence: Evidence,
    /// Where it was found.
    pub source: &'static str,
}

impl<T: Copy> Fact<T> {
    pub const fn measured(value: T, source: &'static str) -> Self {
        Self {
            value,
            evidence: Evidence::Measured,
            source,
        }
    }

    pub const fn derived(value: T, source: &'static str) -> Self {
        Self {
            value,
            evidence: Evidence::Derived,
            source,
        }
    }
}

/// What a game build's world is made of, as far as map size is concerned.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldModel {
    pub name: &'static str,
    /// The edge of one tile, in metres.
    pub tile_m: Fact<u32>,
    /// The metres per tile the New Game page counts when it labels a size.
    pub label_m_per_tile: Fact<u32>,
    /// Base heightmap samples per tile edge: the heightmap is
    /// `tiles * samples + 1` pixels a side.
    pub samples_per_tile: Fact<u32>,
    /// The street raster's stock cell, in metres.
    pub street_cell_m: Fact<u32>,
    /// The octree's root half-extent at `octree_base_depth`, in metres; each
    /// depth past it doubles the extent.
    pub octree_base_half_m: Fact<u32>,
    pub octree_base_depth: u8,
    /// The depth the game picks on its own for maps above
    /// `octree_small_tiles` tiles a side, and the smaller depth below it.
    pub octree_stock_depth: u8,
    pub octree_small_tiles: u32,
    /// The biggest square the stock engine builds.
    pub stock_max_tiles: Fact<u32>,
    /// Generation's peak memory, per km² of map.
    pub generation_mb_per_km2: Fact<f64>,
    /// What the game itself uses on top.
    pub game_base_mb: Fact<u32>,
}

impl WorldModel {
    /// TPF2 build 35924, from silver2127's Big Maps (its README and
    /// `docs/`, summarised in docs/BIGMAPS.md).
    pub const TPF2_BUILD_35924: Self = Self {
        name: "Transport Fever 2 build 35924",
        tile_m: Fact::measured(256, "a 224-tile map reports a 57,344 m bounding box"),
        label_m_per_tile: Fact::measured(
            250,
            "the New Game page labels Megalomaniac 1:1, 96 tiles, as 24 km",
        ),
        samples_per_tile: Fact::measured(64, "heightmap px = tiles * 64 + 1"),
        street_cell_m: Fact::derived(1, "the street raster, RVA 0x90d410"),
        octree_base_half_m: Fact::derived(
            16_384,
            "ecs::OctreeSystem's root box, byte-verified 2026-09-06",
        ),
        octree_base_depth: 9,
        octree_stock_depth: 10,
        octree_small_tiles: 128,
        stock_max_tiles: Fact::measured(224, "the settings.lua override's clamp"),
        generation_mb_per_km2: Fact::measured(
            2.5,
            "\"Terrain toolkit used 10 maps and ... MB\" at 604, 3,288 and 6,711 km²",
        ),
        game_base_mb: Fact::measured(4096, "the game's own use, about 4 GB"),
    };

    /// The octree root's half-extent at `depth`, in metres.
    pub fn octree_half_m(&self, depth: u8) -> u64 {
        let base = u64::from(self.octree_base_half_m.value);
        match depth.checked_sub(self.octree_base_depth) {
            Some(extra) if extra < 32 => base << extra,
            Some(_) => u64::MAX,
            None => base >> (self.octree_base_depth - depth).min(63),
        }
    }

    /// The depth the game picks for a map this size without Big Maps.
    pub fn stock_octree_depth(&self, tiles_x: u32, tiles_y: u32) -> u8 {
        if tiles_x.max(tiles_y) <= self.octree_small_tiles {
            self.octree_base_depth
        } else {
            self.octree_stock_depth
        }
    }

    /// An edge of `tiles` tiles, in metres.
    pub fn edge_m(&self, tiles: u32) -> u64 {
        u64::from(tiles) * u64::from(self.tile_m.value)
    }

    /// A map's area in km².
    pub fn area_km2(&self, tiles_x: u32, tiles_y: u32) -> f64 {
        self.edge_m(tiles_x) as f64 * self.edge_m(tiles_y) as f64 / 1e6
    }

    /// The size the New Game page shows for an edge of `tiles` tiles, in
    /// whole km, as the page itself counts it.
    pub fn label_km(&self, tiles: u32) -> u32 {
        let metres = u64::from(tiles) * u64::from(self.label_m_per_tile.value);
        u32::try_from(metres / 1000).unwrap_or(u32::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TPF2: WorldModel = WorldModel::TPF2_BUILD_35924;

    #[test]
    fn the_octree_root_doubles_with_each_depth() {
        assert_eq!(TPF2.octree_half_m(9), 16_384);
        assert_eq!(
            TPF2.octree_half_m(10),
            32_768,
            "the stock root above 128 tiles"
        );
        assert_eq!(
            TPF2.octree_half_m(11),
            65_536,
            "512 tiles, Big Maps' default"
        );
        assert_eq!(TPF2.octree_half_m(13), 262_144, "2048 tiles");
        assert_eq!(TPF2.octree_half_m(8), 8_192);
    }

    #[test]
    fn the_stock_game_picks_its_depth_by_size() {
        assert_eq!(TPF2.stock_octree_depth(96, 96), 9);
        assert_eq!(TPF2.stock_octree_depth(128, 64), 9);
        assert_eq!(TPF2.stock_octree_depth(224, 224), 10);
    }

    #[test]
    fn sizes_are_labelled_as_the_page_labels_them() {
        assert_eq!(TPF2.label_km(96), 24, "Megalomaniac 1:1 reads 24 km");
        assert_eq!(TPF2.label_km(128), 32);
        assert_eq!(TPF2.label_km(512), 128);
        assert_eq!(TPF2.edge_m(224), 57_344, "the measured bounding box");
        assert!(
            (TPF2.area_km2(96, 96) - 603.98).abs() < 0.01,
            "Megalomaniac, 604 km²"
        );
    }
}
