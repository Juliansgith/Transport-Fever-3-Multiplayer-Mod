//! What a map size costs, and which ceilings it hits.
//!
//! Big Maps met these on TPF2, in the order a growing map meets them
//! (docs/BIGMAPS.md, "The ceilings, in the order they are hit"):
//!
//! 1. **The street raster**: one cell per metre over the whole map, counted
//!    in a 32-bit int, so it overflows past about 180 tiles a side. Big Maps
//!    coarsens the cell until the count fits a budget (`street_raster`);
//!    widening the multiply instead would overflow every index after it.
//! 2. **The octree's root**: a constant box, never derived from the map.
//!    Entities past it are invisible to lookups, and the street builder
//!    stacks duplicate nodes there (the 32,768 m wall). A deeper root
//!    (`octree_depth`) moves it.
//! 3. **The heightmap's pixel count**, `(tiles * 64 + 1)²`, past a 32-bit
//!    int. Derived, never reached: no setting passes it.
//! 4. **Memory**: generation peaks at 2.5 MB per km² on top of the game.
//!
//! A size is checked before it is offered and before a room generates it,
//! so a size the engine cannot build is refused, not discovered.

use crate::config::Config;
use crate::world::WorldModel;

/// A ceiling a size hits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ceiling {
    /// Larger than the settings' `max_tiles`.
    MaxTiles { max_tiles: u32 },
    /// The street raster's cells at the stock cell size, past a 32-bit int.
    StreetRaster { cells: u64 },
    /// The map reaches past the octree root at `depth`.
    OctreeRoot {
        half_m: u64,
        root_half_m: u64,
        depth: u8,
    },
    /// The heightmap's pixel count, past a 32-bit int.
    Heightmap { pixels: u64 },
}

impl Ceiling {
    /// What gets past it, if anything does.
    pub fn remedy(&self) -> &'static str {
        match self {
            Self::MaxTiles { .. } => "raise sizes.max_tiles",
            Self::StreetRaster { .. } => "limits.street_raster = true",
            Self::OctreeRoot { .. } => "a deeper limits.octree_depth",
            Self::Heightmap { .. } => "nothing: the engine cannot build it",
        }
    }
}

/// What a size costs and needs, under the settings.
#[derive(Debug, Clone, PartialEq)]
pub struct SizeReport {
    pub tiles: (u32, u32),
    pub area_km2: f64,
    /// Generation's expected peak, the game's own use included.
    pub peak_mb: f64,
    /// The octree depth this size runs at.
    pub octree_depth: u8,
    /// The street raster's cell, in metres.
    pub street_cell_m: u32,
    /// Ceilings the size hits that the settings get past.
    pub handled: Vec<Ceiling>,
    /// Ceilings the size hits that nothing in the settings gets past.
    pub blocked: Vec<Ceiling>,
}

impl SizeReport {
    /// Whether the engine can build it, under these settings.
    pub fn buildable(&self) -> bool {
        self.blocked.is_empty()
    }
}

const INT_MAX: u64 = i32::MAX as u64;

/// The street raster's cell count at `cell_m`.
fn street_cells(world: &WorldModel, tiles: (u32, u32), cell_m: u32) -> u64 {
    let cell = u64::from(cell_m.max(1));
    world.edge_m(tiles.0).div_ceil(cell) * world.edge_m(tiles.1).div_ceil(cell)
}

/// Checks a map of `tiles_x` by `tiles_y` against `world`'s ceilings under
/// `config`.
pub fn check(world: &WorldModel, config: &Config, tiles_x: u32, tiles_y: u32) -> SizeReport {
    let tiles = (tiles_x, tiles_y);
    let area_km2 = world.area_km2(tiles_x, tiles_y);
    let peak_mb =
        area_km2 * world.generation_mb_per_km2.value + f64::from(world.game_base_mb.value);
    let mut handled = Vec::new();
    let mut blocked = Vec::new();

    let max_tiles = config.sizes.max_tiles;
    if tiles_x.max(tiles_y) > max_tiles {
        blocked.push(Ceiling::MaxTiles { max_tiles });
    }

    // The street raster: coarsen the cell only past the budget, as Big Maps
    // does; below it the switch changes nothing.
    let stock_cell = world.street_cell_m.value;
    let stock_cells = street_cells(world, tiles, stock_cell);
    let mut street_cell_m = stock_cell;
    if config.limits.street_raster {
        let budget = u64::from(config.limits.cell_budget_millions) * 1_000_000;
        while street_cells(world, tiles, street_cell_m) > budget && street_cell_m < 64 {
            street_cell_m += 1;
        }
    }
    if stock_cells > INT_MAX {
        let ceiling = Ceiling::StreetRaster { cells: stock_cells };
        if street_cells(world, tiles, street_cell_m) <= INT_MAX {
            handled.push(ceiling);
        } else {
            blocked.push(ceiling);
        }
    }

    // The octree root: the map is centred on the origin.
    let half_m = world.edge_m(tiles_x.max(tiles_y)) / 2;
    let stock_depth = world.stock_octree_depth(tiles_x, tiles_y);
    let octree_depth = match config.limits.octree_depth {
        0 => stock_depth,
        depth => depth,
    };
    let stock_root = world.octree_half_m(stock_depth);
    if half_m > stock_root {
        let root_half_m = world.octree_half_m(octree_depth);
        let ceiling = Ceiling::OctreeRoot {
            half_m,
            root_half_m: stock_root,
            depth: stock_depth,
        };
        if half_m <= root_half_m {
            handled.push(ceiling);
        } else {
            blocked.push(Ceiling::OctreeRoot {
                half_m,
                root_half_m,
                depth: octree_depth,
            });
        }
    }

    let samples = u64::from(world.samples_per_tile.value);
    let pixels = (u64::from(tiles_x) * samples + 1) * (u64::from(tiles_y) * samples + 1);
    if pixels > INT_MAX {
        blocked.push(Ceiling::Heightmap { pixels });
    }

    SizeReport {
        tiles,
        area_km2,
        peak_mb,
        octree_depth,
        street_cell_m,
        handled,
        blocked,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TPF2: WorldModel = WorldModel::TPF2_BUILD_35924;

    fn config(text: &str) -> Config {
        Config::from_toml(text).unwrap().0
    }

    #[test]
    fn the_stock_engine_builds_up_to_the_street_raster_wall() {
        let stock = Config::default();
        assert!(
            check(&TPF2, &stock, 180, 180).buildable(),
            "the 180-tile wall"
        );
        let past = check(&TPF2, &stock, 182, 182);
        assert!(matches!(past.blocked[..], [Ceiling::StreetRaster { .. }]));
        assert_eq!(past.blocked[0].remedy(), "limits.street_raster = true");
    }

    #[test]
    fn a_coarser_street_raster_gets_past_it() {
        let settings = config("[limits]\nstreet_raster = true");
        let report = check(&TPF2, &settings, 224, 224);
        assert!(report.buildable(), "{report:?}");
        assert!(matches!(report.handled[..], [Ceiling::StreetRaster { .. }]));
        assert_eq!(report.street_cell_m, 2, "57 km at 1 m is 3.3 billion cells");
        // Below the budget the switch changes nothing.
        assert_eq!(check(&TPF2, &settings, 96, 96).street_cell_m, 1);
    }

    #[test]
    fn past_256_tiles_the_octree_root_needs_moving() {
        let raster = config("[limits]\nstreet_raster = true");
        assert!(
            check(&TPF2, &raster, 256, 256).buildable(),
            "exactly the stock root"
        );
        let wall = check(&TPF2, &raster, 320, 320);
        assert!(
            matches!(
                wall.blocked[..],
                [Ceiling::OctreeRoot {
                    half_m: 40_960,
                    root_half_m: 32_768,
                    depth: 10
                }]
            ),
            "{wall:?}"
        );
        let deeper = config("[limits]\nstreet_raster = true\noctree_depth = 11");
        let report = check(&TPF2, &deeper, 320, 320);
        assert!(report.buildable(), "{report:?}");
        assert_eq!(report.octree_depth, 11);
        let too_big = check(
            &TPF2,
            &config("[sizes]\nmax_tiles = 2048\n[limits]\nstreet_raster = true\noctree_depth = 11"),
            640,
            640,
        );
        assert!(
            matches!(too_big.blocked[..], [Ceiling::OctreeRoot { depth: 11, .. }]),
            "{too_big:?}"
        );
    }

    #[test]
    fn the_heightmap_is_a_wall_no_setting_passes() {
        let everything =
            config("[sizes]\nmax_tiles = 2048\n[limits]\nstreet_raster = true\noctree_depth = 13");
        assert!(check(&TPF2, &everything, 722, 722).buildable());
        let report = check(&TPF2, &everything, 800, 800);
        assert!(
            matches!(report.blocked[..], [Ceiling::Heightmap { .. }]),
            "{report:?}"
        );
        assert_eq!(
            report.blocked[0].remedy(),
            "nothing: the engine cannot build it"
        );
    }

    #[test]
    fn memory_follows_the_measured_law() {
        // 16.4 GB for 80 x 80 km in Big Maps' table, plus the game's 4 GB.
        let report = check(&TPF2, &Config::default(), 320, 320);
        assert!((report.area_km2 - 6710.9).abs() < 0.1);
        assert!((report.peak_mb - (6710.9 * 2.5 + 4096.0)).abs() < 1.0);
    }

    #[test]
    fn max_tiles_holds_every_side() {
        let report = check(&TPF2, &Config::default(), 600, 100);
        assert!(
            report
                .blocked
                .contains(&Ceiling::MaxTiles { max_tiles: 512 })
        );
    }
}
