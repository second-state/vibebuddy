//! The buddy's 18×18 pixel face, the same drawing as the Mac app's `PixelFace.swift`. Closed eyes mean the
//! link is down.

pub const SIZE: usize = 18;

pub fn pixels(eyes_closed: bool) -> [[bool; SIZE]; SIZE] {
    let mut grid = [[false; SIZE]; SIZE];
    let mut px = |x: usize, y: usize, w: usize, h: usize| {
        for row in grid.iter_mut().skip(y).take(h) {
            for cell in row.iter_mut().skip(x).take(w) {
                *cell = true;
            }
        }
    };
    // Antenna
    px(8, 0, 2, 2);
    px(8, 2, 2, 1);
    // Head outline
    px(3, 4, 12, 1);
    px(2, 5, 1, 9);
    px(15, 5, 1, 9);
    px(3, 14, 12, 1);
    if eyes_closed {
        px(5, 9, 3, 1);
        px(10, 9, 3, 1);
        px(7, 12, 4, 1);
    } else {
        px(5, 7, 3, 3);
        px(10, 7, 3, 3);
        px(6, 12, 1, 1);
        px(7, 13, 4, 1);
        px(11, 12, 1, 1);
    }
    // Legs
    px(5, 15, 2, 2);
    px(11, 15, 2, 2);
    grid
}

/// ARGB32 in network byte order, as StatusNotifierItem pixmaps want it, scaled up by `scale`.
pub fn argb(eyes_closed: bool, color: [u8; 4], scale: usize) -> Vec<u8> {
    let grid = pixels(eyes_closed);
    let side = SIZE * scale;
    let mut out = Vec::with_capacity(side * side * 4);
    for y in 0..side {
        for x in 0..side {
            let [r, g, b, a] = if grid[y / scale][x / scale] { color } else { [0; 4] };
            out.extend_from_slice(&[a, r, g, b]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_and_closed_eyes_differ() {
        assert!(pixels(false)[8][6]);
        assert!(!pixels(true)[8][6]);
        assert!(pixels(true)[9][6]);
    }

    #[test]
    fn the_pixmap_is_square_and_scaled() {
        let pixmap = argb(false, [255, 255, 255, 255], 2);
        assert_eq!(pixmap.len(), 36 * 36 * 4);
        // The antenna's top-left pixel, (8, 0), is opaque white at 2× scale.
        let offset = (16) * 4;
        assert_eq!(&pixmap[offset..offset + 4], &[255, 255, 255, 255]);
        assert_eq!(&pixmap[0..4], &[0, 0, 0, 0]);
    }
}
