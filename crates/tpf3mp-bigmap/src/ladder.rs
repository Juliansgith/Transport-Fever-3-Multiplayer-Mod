//! The added map sizes: rows appended to the New Game menu's size dropdown,
//! after the game's own, overwriting none of them.
//!
//! As in Big Maps (its `add_size_rows`): each row is a square, labelled by
//! its size in km as the New Game page counts km, and the ratio dropdown
//! shapes it the way the game's own presets are shaped: 1:k keeps about the
//! square's area, the short side being the square's edge over √k and the
//! long side k times that, each an even tile count, the long side capped at
//! the largest size the build allows. On TPF2 the ratio labels could not be
//! changed, since the menu was native; if TF3's is a script recipe, a mod
//! may label them honestly (docs/BIGMAPS.md).

use serde::{Deserialize, Serialize};

use crate::world::WorldModel;

/// The ratio dropdown's entries, as the k of 1:k.
pub const RATIOS: [u32; 5] = [1, 2, 3, 4, 5];

/// One added size.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SizeRow {
    /// What the dropdown shows.
    pub label: String,
    /// The square's edge in tiles: even.
    pub tiles: u32,
}

/// Big Maps' rows: from just above the vanilla menu's biggest square (96
/// tiles) to its octree ceiling (512 tiles), labelled as `world` counts km.
pub fn default_rows(world: &WorldModel) -> Vec<SizeRow> {
    [128, 160, 192, 224, 256, 320, 384, 448, 512]
        .into_iter()
        .map(|tiles| row(world, tiles))
        .collect()
}

/// A square row of `tiles`, labelled as `world` counts km.
pub fn row(world: &WorldModel, tiles: u32) -> SizeRow {
    let km = world.label_km(tiles);
    SizeRow {
        label: format!("{km} x {km} km"),
        tiles,
    }
}

/// The nearest even count to `value`, at least 2.
fn even(value: f64) -> u32 {
    let halves = (value / 2.0).round().max(1.0);
    // The shapes stay far below u32::MAX: rows are capped by max_tiles.
    (halves as u32).saturating_mul(2)
}

/// The shape 1:`k` gives a row of `tiles`, as (long side, short side) in
/// tiles, with the long side capped at `max_tiles`.
pub fn shape(tiles: u32, k: u32, max_tiles: u32) -> (u32, u32) {
    let cap = max_tiles - max_tiles % 2;
    if k <= 1 {
        let side = tiles.min(cap);
        return (side, side);
    }
    let short = even(f64::from(tiles) / f64::from(k).sqrt()).min(cap);
    let long = short.saturating_mul(k).min(cap);
    (long, short)
}

/// Every shape of a row, one per ratio.
pub fn shapes(row: &SizeRow, max_tiles: u32) -> [(u32, u32); 5] {
    RATIOS.map(|k| shape(row.tiles, k, max_tiles))
}

/// The ratios the New Game page adds after the game's own five: what makes
/// long edges reachable. Only the mod's page offers them; the game's
/// `map.format` stays its own last ratio, 1:5.
pub const EXTRA_RATIOS: [u32; 5] = [6, 7, 8, 9, 10];

/// The shape 1:`k` gives a square of `tiles`, as (long side, short side):
/// the square's area kept, the short side even, never capped. A shape
/// longer than the settings allow is not built at all, rather than
/// squeezed into another ratio.
pub fn long_shape(tiles: u32, k: u32) -> (u32, u32) {
    let short = even(f64::from(tiles) / f64::from(k.max(1)).sqrt());
    (short.saturating_mul(k), short)
}

/// A square's shapes at the added ratios.
pub fn extra_shapes(tiles: u32) -> [(u32, u32); 5] {
    EXTRA_RATIOS.map(|k| long_shape(tiles, k))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TPF2: WorldModel = WorldModel::TPF2_BUILD_35924;

    #[test]
    fn the_default_rows_are_big_maps_ladder() {
        let rows = default_rows(&TPF2);
        let labels: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "32 x 32 km",
                "40 x 40 km",
                "48 x 48 km",
                "56 x 56 km",
                "64 x 64 km",
                "80 x 80 km",
                "96 x 96 km",
                "112 x 112 km",
                "128 x 128 km"
            ]
        );
        assert!(rows.iter().all(|r| r.tiles % 2 == 0));
    }

    #[test]
    fn a_ratio_keeps_about_the_squares_area() {
        // Rows whose every shape stays under the cap.
        for tiles in [128, 160, 192] {
            let square = f64::from(tiles * tiles);
            for k in RATIOS {
                let (long, short) = shape(tiles, k, 512);
                assert_eq!(long % 2, 0);
                assert_eq!(short % 2, 0);
                assert_eq!(long, short * k, "1:{k} of {tiles}");
                let area = f64::from(long * short);
                assert!(
                    (area / square - 1.0).abs() < 0.05,
                    "1:{k} of {tiles}: {long}x{short}"
                );
            }
        }
    }

    #[test]
    fn the_added_ratios_keep_the_squares_area_uncapped() {
        // Gigantomaniac's 112: 1:5 is the game's own 50 x 250.
        assert_eq!(long_shape(112, 5), (250, 50));
        assert_eq!(
            extra_shapes(112),
            [(276, 46), (294, 42), (320, 40), (342, 38), (360, 36)]
        );
        // Megalomaniac's 96 at 1:6 stays inside the stock root.
        assert_eq!(long_shape(96, 6), (240, 40));
        for tiles in [32, 96, 176, 384] {
            for (long, short) in extra_shapes(tiles) {
                assert_eq!(short % 2, 0);
                let area = f64::from(long * short) / f64::from(tiles * tiles);
                assert!((area - 1.0).abs() < 0.25, "{tiles}: {long}x{short}");
            }
        }
    }

    #[test]
    fn the_long_side_is_capped() {
        // 448 / sqrt(2) = 316.8, so 316 x 632 before the 512 cap.
        assert_eq!(shape(448, 2, 512), (512, 316));
        assert_eq!(shape(512, 1, 512), (512, 512));
        assert_eq!(
            shape(600, 1, 511),
            (510, 510),
            "an odd cap rounds down to even"
        );
        let row = row(&TPF2, 512);
        assert!(
            shapes(&row, 512)
                .iter()
                .all(|&(long, short)| long <= 512 && short <= long)
        );
    }
}
