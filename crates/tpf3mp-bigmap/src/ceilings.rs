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
    /// Placement's farthest squared separation, in heightmap pixels, past
    /// a 32-bit int: pairs farther apart than 185 km wrap.
    PlacementSpacing { pixels_squared: u64 },
}

impl Ceiling {
    /// What gets past it, if anything does.
    pub fn remedy(&self) -> &'static str {
        match self {
            Self::MaxTiles { .. } => "raise sizes.max_tiles",
            Self::StreetRaster { .. } => "limits.street_raster = true",
            Self::OctreeRoot { .. } => "a deeper limits.octree_depth",
            Self::Heightmap { .. } => "nothing: the engine cannot build it",
            Self::PlacementSpacing { .. } => "limits.placement_distance = true",
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

/// The street raster's cell count at `cell_m`, as the raster's constructor
/// counts a side: `floor(extent / cell) + 1`.
fn street_cells(world: &WorldModel, tiles: (u32, u32), cell_m: u32) -> u64 {
    let cell = u64::from(cell_m.max(1));
    (world.edge_m(tiles.0) / cell + 1) * (world.edge_m(tiles.1) / cell + 1)
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
    if config.limits.street_raster && world.patches_from_world {
        // TF3's patch (crates/tpf3mp-hook/src/bigmap/raster.rs): only where
        // the stock cell overflows, doubled until the count fits.
        if stock_cells > INT_MAX {
            street_cell_m = stock_cell * 2;
            while street_cells(world, tiles, street_cell_m) > INT_MAX && street_cell_m < 64 {
                street_cell_m *= 2;
            }
        }
    } else if config.limits.street_raster {
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
    let stock_root = world.octree_half_m(stock_depth);
    // A depth past what the build's patches reach is not there to use.
    let wanted = match config.limits.octree_depth {
        0 => stock_depth,
        depth => depth.min(world.octree_max_depth.value),
    };
    // Derived from the world (TF3): the game's own root wherever it covers
    // the map, so a stock-sized world runs stock code.
    let octree_depth = if world.patches_from_world && half_m <= stock_root {
        stock_depth
    } else {
        wanted
    };
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

    // Placement's farthest pair: corner to corner, in heightmap pixels.
    let samples = u64::from(world.samples_per_tile.value);
    let (px, py) = (u64::from(tiles_x) * samples, u64::from(tiles_y) * samples);
    let pixels_squared = px * px + py * py;
    if pixels_squared > INT_MAX {
        let ceiling = Ceiling::PlacementSpacing { pixels_squared };
        // Big Maps on TPF2 fixes it on every Steam load, unasked.
        if config.limits.placement_distance || !world.patches_from_world {
            handled.push(ceiling);
        } else {
            blocked.push(ceiling);
        }
    }

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

    const TF3: WorldModel = WorldModel::TF3_BUILD_40408;

    #[test]
    fn tf3_moves_the_root_only_for_worlds_that_need_it() {
        let depth11 = config("[limits]\noctree_depth = 11");
        // Stock and stage 1 sizes keep the game's own root.
        assert_eq!(check(&TF3, &depth11, 112, 112).octree_depth, 9);
        assert_eq!(check(&TF3, &depth11, 50, 250).octree_depth, 10);
        assert_eq!(check(&TF3, &depth11, 176, 176).octree_depth, 10);
        assert_eq!(check(&TF3, &depth11, 2, 256).octree_depth, 10);
        // Past 256 tiles: depth 11, up to 512.
        let long = check(&TF3, &depth11, 60, 300);
        assert!(long.buildable(), "{long:?}");
        assert_eq!(long.octree_depth, 11);
        assert!(matches!(
            long.handled[..],
            [Ceiling::OctreeRoot { depth: 10, .. }]
        ));
        assert!(check(&TF3, &depth11, 62, 512).buildable());
        // Without the setting the stock root blocks it.
        let stock = check(&TF3, &Config::default(), 60, 300);
        assert!(matches!(
            stock.blocked[..],
            [Ceiling::OctreeRoot { depth: 10, .. }]
        ));
    }

    #[test]
    fn tf3_has_no_depth_past_11_yet() {
        assert_eq!(TF3.octree_max_depth.value, 11);
        // 1,000 tiles (256 km) needs depth 12: blocked at 11 whatever the
        // settings ask.
        for depth in [11, 12, 13] {
            let settings = config(&format!(
                "[sizes]\nmax_tiles = 1000\n[limits]\nstreet_raster = true\nplacement_distance = true\noctree_depth = {depth}"
            ));
            let report = check(&TF3, &settings, 40, 1000);
            assert!(
                matches!(
                    report.blocked[..],
                    [Ceiling::OctreeRoot {
                        half_m: 128_000,
                        root_half_m: 65_536,
                        depth: 11
                    }]
                ),
                "{depth}: {report:?}"
            );
        }
        // TPF2's Big Maps reached 13.
        let tpf2 =
            config("[sizes]\nmax_tiles = 1024\n[limits]\nstreet_raster = true\noctree_depth = 12");
        assert!(check(&TPF2, &tpf2, 40, 1000).buildable());
    }

    #[test]
    fn tf3_grows_the_street_cell_only_where_1_m_overflows() {
        let raster = config("[limits]\nstreet_raster = true\noctree_depth = 11");
        // 176² fits at 1 m (Big Maps' 1.5-billion budget would have grown it).
        assert_eq!(check(&TF3, &raster, 176, 176).street_cell_m, 1);
        assert_eq!(check(&TF3, &raster, 180, 180).street_cell_m, 1);
        let wall = check(&TF3, &raster, 182, 182);
        assert!(wall.buildable(), "{wall:?}");
        assert_eq!(wall.street_cell_m, 2);
        assert_eq!(check(&TF3, &raster, 300, 300).street_cell_m, 2);
        assert_eq!(check(&TF3, &raster, 512, 512).street_cell_m, 4);
        // The memory table's 256 km shapes (with depth 12, not built).
        let long = config(
            "[sizes]\nmax_tiles = 1000\n[limits]\nstreet_raster = true\nplacement_distance = true",
        );
        for (short, cell) in [(32, 1), (40, 2), (88, 2), (130, 2), (136, 4), (186, 4)] {
            assert_eq!(
                check(&TF3, &long, short, 1000).street_cell_m,
                cell,
                "{short}"
            );
        }
        // Without the patch past 180²: blocked.
        let stock = check(&TF3, &config("[limits]\noctree_depth = 11"), 182, 182);
        assert!(matches!(stock.blocked[..], [Ceiling::StreetRaster { .. }]));
    }

    #[test]
    fn tf3_spacing_wraps_only_past_185_km() {
        let depth11 = config("[limits]\noctree_depth = 11\nstreet_raster = true");
        // 390 x 78 (stage 2's longest): 100 km apart at most.
        assert!(check(&TF3, &depth11, 390, 78).buildable());
        // 512 x 512's corners are 2^31 px² apart: one past INT_MAX.
        let corner = check(&TF3, &depth11, 512, 512);
        assert_eq!(
            corner.blocked,
            [Ceiling::PlacementSpacing {
                pixels_squared: 1 << 31
            }]
        );
        assert_eq!(
            corner.blocked[0].remedy(),
            "limits.placement_distance = true"
        );
        let fixed =
            config("[limits]\noctree_depth = 11\nstreet_raster = true\nplacement_distance = true");
        assert!(check(&TF3, &fixed, 512, 512).buildable());
        // Every 1,000-tile map has pairs 256 km apart.
        assert!(
            check(&TF3, &depth11, 2, 1000)
                .blocked
                .iter()
                .any(|c| matches!(c, Ceiling::PlacementSpacing { .. }))
        );
    }

    #[test]
    fn every_stage3_shape_is_buildable_on_tf3() {
        let (settings, notes) =
            Config::from_toml(include_str!("../tpf3mp_bigmap.stage3.toml")).unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        let mut coarse = false;
        for row in settings.rows(&TF3) {
            for (x, y) in crate::ladder::shapes(&row, settings.sizes.max_tiles) {
                let report = check(&TF3, &settings, x, y);
                assert!(report.buildable(), "{} {x}x{y}: {report:?}", row.label);
                coarse |= report.street_cell_m > 1;
            }
        }
        assert!(coarse, "stage 3 has rows past the 1 m raster");
    }

    #[test]
    fn every_stage2_shape_is_buildable_on_tf3() {
        let (settings, notes) =
            Config::from_toml(include_str!("../tpf3mp_bigmap.stage2.toml")).unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        let mut longest = 0;
        for row in settings.rows(&TF3) {
            for (x, y) in crate::ladder::shapes(&row, settings.sizes.max_tiles) {
                let report = check(&TF3, &settings, x, y);
                assert!(report.buildable(), "{} {x}x{y}: {report:?}", row.label);
                assert_eq!(report.street_cell_m, 1, "no raster patch at stage 2");
                longest = longest.max(x.max(y));
            }
        }
        assert!(
            longest > 256,
            "stage 2 reaches past the stock root: {longest}"
        );
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
