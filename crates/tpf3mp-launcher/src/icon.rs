//! The window's icon, drawn rather than shipped as an image: a length of
//! railway track on a blue tile.

use eframe::egui::IconData;

const SIZE: u32 = 64;

/// The icon, as RGBA pixels.
pub fn icon() -> IconData {
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            rgba.extend_from_slice(&pixel(x, y));
        }
    }
    IconData {
        rgba,
        width: SIZE,
        height: SIZE,
    }
}

fn pixel(x: u32, y: u32) -> [u8; 4] {
    const TILE: [u8; 4] = [38, 110, 196, 255];
    const RAIL: [u8; 4] = [236, 240, 245, 255];
    const SLEEPER: [u8; 4] = [120, 84, 52, 255];
    const CLEAR: [u8; 4] = [0, 0, 0, 0];
    // Rounded corners.
    let radius = 12i64;
    let (cx, cy) = (i64::from(x), i64::from(y));
    let edge = i64::from(SIZE) - 1 - radius;
    let dx = (radius - cx).max(cx - edge).max(0);
    let dy = (radius - cy).max(cy - edge).max(0);
    if dx * dx + dy * dy > radius * radius {
        return CLEAR;
    }
    let rails = (22..=25).contains(&y) || (38..=41).contains(&y);
    let sleeper = (18..=45).contains(&y) && x % 10 < 4 && (6..=57).contains(&x);
    if rails && (4..=59).contains(&x) {
        RAIL
    } else if sleeper {
        SLEEPER
    } else {
        TILE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icon_is_square_and_complete() {
        let icon = icon();
        assert_eq!((icon.width, icon.height), (SIZE, SIZE));
        assert_eq!(icon.rgba.len(), (SIZE * SIZE * 4) as usize);
        // Transparent in the corner, opaque in the middle.
        assert_eq!(icon.rgba[3], 0);
        let middle = ((32 * SIZE + 32) * 4 + 3) as usize;
        assert_eq!(icon.rgba[middle], 255);
    }
}
