//! A Character's look: four still key frames that the display moves and marks for each state
//! (docs/characters.md). The robot is not a look; it is the built-in default, drawn in code.
//!
//! Stored inside the Character pack, built by `tools/make-look.py`, little endian:
//! ```text
//!   0  magic "LOOK"
//!   4  u8 width (48), u8 height (64), u8 frame count (4), u8 zero
//!   8  u16[16] palette, RGB565; entry 0 is transparent
//!  40  frames, in the order of [`Frame`]: width × height pixels each, 4 bits per pixel, row by
//!      row, the left pixel of each pair in the low nibble
//! ```

use alloc::boxed::Box;

use crate::canvas::Canvas;

pub const WIDTH: i32 = 48;
pub const HEIGHT: i32 = 64;
/// Each look pixel is drawn as a 2 × 2 block.
pub const SCALE: i32 = 2;
pub const FRAMES: usize = 4;
pub const COLORS: usize = 16;
const FRAME_BYTES: usize = (WIDTH * HEIGHT / 2) as usize;
const HEADER_BYTES: usize = 8 + COLORS * 2;
pub const BYTES: usize = HEADER_BYTES + FRAMES * FRAME_BYTES;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frame {
    Normal = 0,
    EyesClosed = 1,
    Happy = 2,
    Sad = 3,
}

pub struct Look {
    palette: [u16; COLORS],
    frames: [[u8; FRAME_BYTES]; FRAMES],
}

/// RGB565 to the gray of the same brightness, for a lost link.
fn gray(color: u16) -> u16 {
    let red = (color >> 11) as u32 * 255 / 31;
    let green = ((color >> 5) & 0x3F) as u32 * 255 / 63;
    let blue = (color & 0x1F) as u32 * 255 / 31;
    // Darker than the color itself, so a gray buddy reads as switched off rather than washed out.
    let level = (red * 77 + green * 150 + blue * 29) >> 9;
    ((level >> 3) << 11 | (level >> 2) << 5 | (level >> 3)) as u16
}

impl Look {
    /// Parses a look; None if the magic, the size or the frame layout isn't the one this firmware draws.
    pub fn parse(bytes: &[u8]) -> Option<Box<Look>> {
        if bytes.len() != BYTES || &bytes[..4] != b"LOOK" {
            return None;
        }
        if bytes[4] as i32 != WIDTH || bytes[5] as i32 != HEIGHT || bytes[6] as usize != FRAMES {
            return None;
        }
        let mut look = Box::new(Look { palette: [0; COLORS], frames: [[0; FRAME_BYTES]; FRAMES] });
        for (index, color) in look.palette.iter_mut().enumerate() {
            *color = u16::from_le_bytes([bytes[8 + index * 2], bytes[9 + index * 2]]);
        }
        for (index, frame) in look.frames.iter_mut().enumerate() {
            let start = HEADER_BYTES + index * FRAME_BYTES;
            frame.copy_from_slice(&bytes[start..start + FRAME_BYTES]);
        }
        Some(look)
    }

    fn index(&self, frame: Frame, x: i32, y: i32) -> u8 {
        let at = (y * WIDTH + x) as usize;
        let byte = self.frames[frame as usize][at / 2];
        if at.is_multiple_of(2) { byte & 15 } else { byte >> 4 }
    }

    /// Draws a frame with its top-left corner at (left, top) on screen, at 2×. `mirror` flips it
    /// left to right; `gray` draws it in grays.
    pub fn draw(&self, canvas: &mut Canvas, frame: Frame, left: i32, top: i32, mirror: bool, gray_out: bool) {
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let index = self.index(frame, if mirror { WIDTH - 1 - x } else { x }, y);
                if index == 0 {
                    continue;
                }
                let color = self.palette[index as usize];
                let color = if gray_out { gray(color) } else { color };
                canvas.fill_rect(left + x * SCALE, top + y * SCALE, SCALE, SCALE, color);
            }
        }
    }

    /// The topmost row any frame paints, so marks such as `!?` sit just above the head.
    pub fn head_row(&self) -> i32 {
        (0..HEIGHT)
            .find(|&y| (0..WIDTH).any(|x| (0..FRAMES).any(|f| self.index(FRAME_ORDER[f], x, y) != 0)))
            .unwrap_or(0)
    }
}

const FRAME_ORDER: [Frame; FRAMES] = [Frame::Normal, Frame::EyesClosed, Frame::Happy, Frame::Sad];

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::canvas::FRAME_BYTES as SCREEN_BYTES;
    use std::vec;
    use std::vec::Vec;

    /// A look whose frame f is filled with color f + 1 from row `top` down; the palette is distinct
    /// reds, so a drawn pixel says which frame and color it came from.
    pub(crate) fn sample(top: i32) -> Vec<u8> {
        let mut bytes = vec![0u8; BYTES];
        bytes[..4].copy_from_slice(b"LOOK");
        bytes[4..8].copy_from_slice(&[WIDTH as u8, HEIGHT as u8, FRAMES as u8, 0]);
        for index in 0..COLORS {
            bytes[8 + index * 2..10 + index * 2].copy_from_slice(&((index as u16) << 11).to_le_bytes());
        }
        for frame in 0..FRAMES {
            let color = frame as u8 + 1;
            for y in top..HEIGHT {
                // The left half only, so mirroring shows.
                for x in 0..WIDTH / 2 {
                    let at = HEADER_BYTES + frame * FRAME_BYTES + ((y * WIDTH + x) / 2) as usize;
                    bytes[at] |= if x % 2 == 0 { color } else { color << 4 };
                }
            }
        }
        bytes
    }

    fn pixel(screen: &[u8], x: i32, y: i32) -> u16 {
        let at = ((y * crate::canvas::WIDTH + x) * 2) as usize;
        u16::from_be_bytes([screen[at], screen[at + 1]])
    }

    #[test]
    fn a_look_draws_at_twice_its_size_and_skips_transparent_pixels() {
        let look = Look::parse(&sample(10)).expect("parses");
        assert_eq!(look.head_row(), 10);
        let mut screen = vec![0u8; SCREEN_BYTES];
        let mut canvas = Canvas::new(&mut screen);
        look.draw(&mut canvas, Frame::Happy, 100, 20, false, false);
        // Frame Happy is color 3; its left half is painted from row 10 down.
        assert_eq!(pixel(&screen, 100, 20 + 20), 3 << 11);
        assert_eq!(pixel(&screen, 101, 21 + 20), 3 << 11);
        assert_eq!(pixel(&screen, 100, 20 + 18), 0, "transparent above the head");
        assert_eq!(pixel(&screen, 100 + 2 * WIDTH - 1, 20 + 40), 0, "the right half is empty");

        let mut screen = vec![0u8; SCREEN_BYTES];
        let mut canvas = Canvas::new(&mut screen);
        look.draw(&mut canvas, Frame::Normal, 100, 20, true, false);
        assert_eq!(pixel(&screen, 100, 60), 0, "mirrored: the left is empty now");
        assert_eq!(pixel(&screen, 100 + 2 * WIDTH - 1, 60), 1 << 11);
    }

    #[test]
    fn gray_keeps_brightness_order_and_loses_color() {
        let white = gray(0xFFFF);
        let red = gray(0xF800);
        assert!(white > red);
        let (r, g, b) = (red >> 11, (red >> 5) & 0x3F, red & 0x1F);
        assert!(g / 2 == r && r == b, "a gray: {r} {g} {b}");
    }

    #[test]
    fn a_foreign_look_is_refused() {
        let mut bytes = sample(0);
        bytes[4] = 64;
        assert!(Look::parse(&bytes).is_none(), "another width");
        assert!(Look::parse(&sample(0)[..BYTES - 1]).is_none(), "short");
        let mut bytes = sample(0);
        bytes[0] = b'X';
        assert!(Look::parse(&bytes).is_none(), "magic");
    }
}
