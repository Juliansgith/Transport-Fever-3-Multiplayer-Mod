//! Transport Fever 3's own map sizes, and the walls stock TF3 stays inside.
//!
//! The sizes are `getNumTiles` in the game's
//! `gui/menu/new_game_or_map_settings_page.tl` (build 40408, lines 17-77):
//! eight size rows, five ratios each, in tiles. The walls are those
//! investigation/TF3_BIGMAPS_PORT_2026-10-01.md section 1.4 derives from
//! the executable; a size inside all of them needs no native patch, so a
//! world of that size is safe in any game, with or without big maps.

use crate::world::WorldModel;

/// One of the game's size rows: its name in the code and its shape at each
/// ratio, 1:1 to 1:5, as (x, y) tiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StockSize {
    pub name: &'static str,
    pub shapes: [(u32, u32); 5],
}

/// `getNumTiles`, build 40408. Tiny and Small repeat an entry at 1:4,
/// as the game does.
pub const TF3_BUILD_40408: [StockSize; 8] = [
    StockSize {
        name: "tiny",
        shapes: [(16, 16), (10, 20), (8, 24), (8, 24), (6, 30)],
    },
    StockSize {
        name: "small",
        shapes: [(32, 32), (22, 44), (18, 54), (16, 54), (14, 70)],
    },
    StockSize {
        name: "medium",
        shapes: [(44, 44), (32, 64), (26, 78), (22, 88), (20, 100)],
    },
    StockSize {
        name: "large",
        shapes: [(56, 56), (40, 80), (32, 96), (28, 112), (24, 126)],
    },
    StockSize {
        name: "veryLarge",
        shapes: [(64, 64), (44, 88), (36, 108), (32, 128), (28, 140)],
    },
    StockSize {
        name: "huge",
        shapes: [(80, 80), (56, 112), (46, 138), (40, 160), (34, 170)],
    },
    StockSize {
        name: "megalomaniac",
        shapes: [(96, 96), (66, 132), (54, 162), (48, 192), (42, 210)],
    },
    StockSize {
        name: "colossal",
        shapes: [(112, 112), (80, 160), (64, 192), (56, 224), (50, 250)],
    },
];

/// The longest axis stock TF3 builds: Gigantomaniac ("colossal") at 1:5.
pub const LONGEST_AXIS: u32 = 250;

/// The street/obstacle raster's cells for a map at a one-metre cell:
/// `(edge_x + 1) * (edge_y + 1)`, which the game multiplies in 32 bits.
pub fn raster_cells(world: &WorldModel, tiles_x: u32, tiles_y: u32) -> u64 {
    (world.edge_m(tiles_x) + 1) * (world.edge_m(tiles_y) + 1)
}

/// Whether a size stays inside every wall stock TF3 stays inside: no axis
/// longer than [`LONGEST_AXIS`] (the octree root keeps stock's margin) and
/// the raster's cells below 2^31.
pub fn inside_stock_walls(world: &WorldModel, tiles_x: u32, tiles_y: u32) -> bool {
    tiles_x.max(tiles_y) <= LONGEST_AXIS && raster_cells(world, tiles_x, tiles_y) <= i32::MAX as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    const TF3: WorldModel = WorldModel::TF3_BUILD_40408;

    #[test]
    fn every_stock_size_is_inside_the_stock_walls() {
        for size in TF3_BUILD_40408 {
            for (x, y) in size.shapes {
                assert!(inside_stock_walls(&TF3, x, y), "{} {x}x{y}", size.name);
                assert!(x <= y, "the game's shapes put the short side first");
            }
        }
        let longest = TF3_BUILD_40408
            .iter()
            .flat_map(|size| size.shapes)
            .map(|(x, y)| x.max(y))
            .max();
        assert_eq!(longest, Some(LONGEST_AXIS));
    }

    #[test]
    fn the_raster_wall_is_at_180_tiles_square() {
        // 46,081² = 2.12e9 cells fits; 182 tiles is 46,593² = 2.17e9.
        assert!(inside_stock_walls(&TF3, 180, 180));
        assert!(!inside_stock_walls(&TF3, 182, 182));
        assert!(!inside_stock_walls(&TF3, 52, 252), "the axis wall");
    }
}
