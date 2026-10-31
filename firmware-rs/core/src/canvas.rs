//! A 320×240 RGB565 frame buffer and the most basic drawing: filled rectangles, thick lines, 5×7 bitmap text,
//! and UTF-8 text that adds Chinese from the 12 px font in font.rs.
//!
//! The buffer is in the byte order the panel wants: high byte first for each pixel. The C firmware stores
//! little-endian u16 in memory and has the LCD driver swap bytes on send; here it is stored as it goes on
//! the wire, so DMA can send it as is.

pub const WIDTH: i32 = 320;
pub const HEIGHT: i32 = 240;
pub const FRAME_BYTES: usize = (WIDTH * HEIGHT * 2) as usize;

use crate::font::{self, Glyph};

const LETTER_GLYPHS: [[u8; 7]; 26] = [
    [0x0e, 0x11, 0x11, 0x1f, 0x11, 0x11, 0x11],
    [0x1e, 0x11, 0x11, 0x1e, 0x11, 0x11, 0x1e],
    [0x0e, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0e],
    [0x1e, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1e],
    [0x1f, 0x10, 0x10, 0x1e, 0x10, 0x10, 0x1f],
    [0x1f, 0x10, 0x10, 0x1e, 0x10, 0x10, 0x10],
    [0x0e, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0f],
    [0x11, 0x11, 0x11, 0x1f, 0x11, 0x11, 0x11],
    [0x1f, 0x04, 0x04, 0x04, 0x04, 0x04, 0x1f],
    [0x07, 0x02, 0x02, 0x02, 0x12, 0x12, 0x0c],
    [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
    [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1f],
    [0x11, 0x1b, 0x15, 0x15, 0x11, 0x11, 0x11],
    [0x11, 0x19, 0x15, 0x13, 0x11, 0x11, 0x11],
    [0x0e, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0e],
    [0x1e, 0x11, 0x11, 0x1e, 0x10, 0x10, 0x10],
    [0x0e, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0d],
    [0x1e, 0x11, 0x11, 0x1e, 0x14, 0x12, 0x11],
    [0x0f, 0x10, 0x10, 0x0e, 0x01, 0x01, 0x1e],
    [0x1f, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
    [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0e],
    [0x11, 0x11, 0x11, 0x11, 0x11, 0x0a, 0x04],
    [0x11, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0a],
    [0x11, 0x11, 0x0a, 0x04, 0x0a, 0x11, 0x11],
    [0x11, 0x11, 0x0a, 0x04, 0x04, 0x04, 0x04],
    [0x1f, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1f],
];

const DIGIT_GLYPHS: [[u8; 7]; 10] = [
    [0x0e, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0e],
    [0x04, 0x0c, 0x04, 0x04, 0x04, 0x04, 0x0e],
    [0x0e, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1f],
    [0x1e, 0x01, 0x01, 0x0e, 0x01, 0x01, 0x1e],
    [0x02, 0x06, 0x0a, 0x12, 0x1f, 0x02, 0x02],
    [0x1f, 0x10, 0x10, 0x1e, 0x01, 0x01, 0x1e],
    [0x0e, 0x10, 0x10, 0x1e, 0x11, 0x11, 0x0e],
    [0x1f, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
    [0x0e, 0x11, 0x11, 0x0e, 0x11, 0x11, 0x0e],
    [0x0e, 0x11, 0x11, 0x0f, 0x01, 0x01, 0x0e],
];

/// One bitmap row for a byte. Bytes without a glyph (including every byte of a UTF-8 multibyte character)
/// draw as a small question-mark-like placeholder; drawing is per byte, as in the C firmware.
fn glyph_row(character: u8, row: usize) -> u8 {
    let upper = character.to_ascii_uppercase();
    match upper {
        b'A'..=b'Z' => LETTER_GLYPHS[(upper - b'A') as usize][row],
        b'0'..=b'9' => DIGIT_GLYPHS[(upper - b'0') as usize][row],
        b'-' => if row == 3 { 0x1f } else { 0 },
        b'.' => if row == 6 { 0x04 } else { 0 },
        b',' => match row { 5 => 0x04, 6 => 0x08, _ => 0 },
        b'\'' => if row < 2 { 0x04 } else { 0 },
        b':' => if row == 2 || row == 5 { 0x04 } else { 0 },
        b'/' => 1 << if row < 5 { 4 - row } else { 0 },
        b'!' => if row < 5 || row == 6 { 0x04 } else { 0 },
        b'?' => [0x0e, 0x11, 0x01, 0x02, 0x04, 0x00, 0x04][row],
        b'>' => [0x10, 0x08, 0x04, 0x02, 0x04, 0x08, 0x10][row],
        b' ' => 0,
        b'_' => if row == 6 { 0x1f } else { 0 },
        b'%' => [0x18, 0x19, 0x02, 0x04, 0x08, 0x13, 0x03][row],
        _ => match row {
            0 | 3 => 0x0e,
            1 | 2 => 0x11,
            _ => 0x04,
        },
    }
}

/// Halves each RGB565 component.
pub fn half_bright(color: u16) -> u16 {
    (color >> 1) & 0x7bef
}

pub struct Canvas<'a> {
    bytes: &'a mut [u8],
}

impl<'a> Canvas<'a> {
    pub fn new(bytes: &'a mut [u8]) -> Self {
        assert!(bytes.len() >= FRAME_BYTES);
        Self { bytes }
    }

    pub fn pixel(&self, index: usize) -> u16 {
        u16::from_be_bytes([self.bytes[index * 2], self.bytes[index * 2 + 1]])
    }

    pub fn fill_rect(&mut self, x: i32, y: i32, width: i32, height: i32, color: u16) {
        let x_start = x.max(0);
        let y_start = y.max(0);
        let x_end = (x + width).min(WIDTH);
        let y_end = (y + height).min(HEIGHT);
        if x_start >= x_end || y_start >= y_end {
            return;
        }
        let [high, low] = color.to_be_bytes();
        for row in y_start..y_end {
            let start = ((row * WIDTH + x_start) * 2) as usize;
            let end = ((row * WIDTH + x_end) * 2) as usize;
            for pair in self.bytes[start..end].as_chunks_mut::<2>().0 {
                pair[0] = high;
                pair[1] = low;
            }
        }
    }

    /// Mixes `color` into the pixel at (x, y) by `alpha` sixteenths (0 leaves it, 15 replaces it); the korekore
    /// screens draw anti-aliased type this way.
    pub fn blend(&mut self, x: i32, y: i32, color: u16, alpha: u8) {
        if alpha == 0 || !(0..WIDTH).contains(&x) || !(0..HEIGHT).contains(&y) {
            return;
        }
        let at = ((y * WIDTH + x) * 2) as usize;
        let mixed = if alpha >= 15 {
            color
        } else {
            let under = u16::from_be_bytes([self.bytes[at], self.bytes[at + 1]]);
            let mix = |shift: u32, mask: u16| {
                let (top, bottom) = (u32::from((color >> shift) & mask), u32::from((under >> shift) & mask));
                ((top * u32::from(alpha) + bottom * (15 - u32::from(alpha)) + 7) / 15) as u16 & mask
            };
            mix(11, 0x1f) << 11 | mix(5, 0x3f) << 5 | mix(0, 0x1f)
        };
        self.bytes[at..at + 2].copy_from_slice(&mixed.to_be_bytes());
    }

    /// A three-pixel-thick Bresenham line.
    pub fn draw_line(&mut self, mut x0: i32, mut y0: i32, x1: i32, y1: i32, color: u16) {
        let delta_x = (x1 - x0).abs();
        let step_x = if x0 < x1 { 1 } else { -1 };
        let delta_y = -(y1 - y0).abs();
        let step_y = if y0 < y1 { 1 } else { -1 };
        let mut error = delta_x + delta_y;
        loop {
            self.fill_rect(x0 - 1, y0 - 1, 3, 3, color);
            if x0 == x1 && y0 == y1 {
                break;
            }
            let doubled = 2 * error;
            if doubled >= delta_y {
                error += delta_y;
                x0 += step_x;
            }
            if doubled <= delta_x {
                error += delta_x;
                y0 += step_y;
            }
        }
    }

    /// Draws byte by byte from (x, y), at most `max_characters`; stops at NUL.
    pub fn draw_text(&mut self, x: i32, y: i32, text: &[u8], scale: i32, color: u16, max_characters: usize) {
        for (index, &character) in text.iter().take(max_characters).enumerate() {
            if character == 0 {
                break;
            }
            self.draw_ascii(x + index as i32 * 6 * scale, y, character, scale, color);
        }
    }

    fn draw_ascii(&mut self, x: i32, y: i32, character: u8, scale: i32, color: u16) {
        for row in 0..7 {
            let bits = glyph_row(character, row);
            for column in 0..5 {
                if bits & (1 << (4 - column)) != 0 {
                    self.fill_rect(x + column * scale, y + row as i32 * scale, scale, scale, color);
                }
            }
        }
    }

    /// Draws UTF-8 text from (x, y), as many whole characters as fit in `max_width` pixels; stops at NUL.
    /// ASCII is the 5×7 font exactly as `draw_text` draws it, with (x, y) its top left; Chinese is the 12 px
    /// font, centred on the ASCII line (font.rs). Returns the width drawn.
    pub fn draw_str(&mut self, x: i32, y: i32, text: &[u8], scale: i32, color: u16, max_width: i32) -> i32 {
        let text = &text[..font::fit(text, scale, max_width)];
        let wide_scale = font::wide_scale(scale);
        let top = font::wide_top(y, scale);
        let mut left = x;
        for (glyph, _) in font::glyphs(text) {
            match glyph {
                Glyph::Ascii(character) => self.draw_ascii(left, y, character, scale, color),
                Glyph::Wide(index) => {
                    for row in 0..font::CELL {
                        let bits = font::row(index, row as usize);
                        for column in 0..font::CELL {
                            if bits & (0x800 >> column) != 0 {
                                self.fill_rect(left + column * wide_scale, top + row * wide_scale, wide_scale, wide_scale, color);
                            }
                        }
                    }
                }
                // A dotted box over the ink area, so it can't pass for 口.
                Glyph::Missing => {
                    for step in (0..11).step_by(2) {
                        for (column, row) in [(step, 1), (step, 11), (0, 1 + step), (10, 1 + step)] {
                            self.fill_rect(left + column * wide_scale, top + row * wide_scale, wide_scale, wide_scale, color);
                        }
                    }
                }
            }
            left += font::advance(glyph, scale);
        }
        font::width(text, scale)
    }

    /// `draw_str` centred across the screen.
    pub fn draw_str_centered(&mut self, y: i32, text: &[u8], scale: i32, color: u16) {
        // As much as `draw_text_centered` allows, so ASCII lands where it always has.
        let max_width = WIDTH / (6 * scale) * 6 * scale - scale;
        let text = &text[..font::fit(text, scale, max_width)];
        self.draw_str((WIDTH - font::width(text, scale)) / 2, y, text, scale, color, max_width);
    }

    pub fn draw_text_centered(&mut self, y: i32, text: &[u8], scale: i32, color: u16) {
        let max_characters = (WIDTH / (6 * scale)) as usize;
        let length = text.iter().take(max_characters).take_while(|&&byte| byte != 0).count() as i32;
        let width = if length == 0 { 0 } else { (length * 6 - 1) * scale };
        self.draw_text((WIDTH - width) / 2, y, text, scale, color, max_characters);
    }

    /// Dims the whole frame for the sleepy phase: halves each component of every pixel.
    pub fn dim(&mut self) {
        for pair in self.bytes[..FRAME_BYTES].as_chunks_mut::<2>().0 {
            let dimmed = half_bright(u16::from_be_bytes([pair[0], pair[1]])).to_be_bytes();
            pair.copy_from_slice(&dimmed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    fn render(draw: impl FnOnce(&mut Canvas)) -> Vec<u8> {
        let mut bytes = vec![0; FRAME_BYTES];
        draw(&mut Canvas::new(&mut bytes));
        bytes
    }

    /// The lit area as (top, bottom, left, right), inclusive.
    fn ink(bytes: &[u8]) -> (i32, i32, i32, i32) {
        let lit: Vec<(i32, i32)> =
            (0..WIDTH * HEIGHT).filter(|&index| bytes[index as usize * 2] != 0).map(|index| (index / WIDTH, index % WIDTH)).collect();
        let rows = || lit.iter().map(|&(row, _)| row);
        let columns = || lit.iter().map(|&(_, column)| column);
        (rows().min().unwrap(), rows().max().unwrap(), columns().min().unwrap(), columns().max().unwrap())
    }

    #[test]
    fn ascii_draws_the_same_through_either_call() {
        let text = b"Ready: 3 TASKS, 12m_ok?\x01~";
        for scale in 1..=3 {
            let old = render(|canvas| canvas.draw_text(4, 30, text, scale, 0xffff, 40));
            let new = render(|canvas| {
                canvas.draw_str(4, 30, text, scale, 0xffff, 40 * 6 * scale - scale);
            });
            assert!(old == new, "scale {scale}");
            let old = render(|canvas| canvas.draw_text_centered(30, text, scale, 0xffff));
            let new = render(|canvas| canvas.draw_str_centered(30, text, scale, 0xffff));
            assert!(old == new, "centred, scale {scale}");
        }
    }

    #[test]
    fn chinese_sits_on_the_ascii_line() {
        // Scale 1: the 5x7 glyph covers rows 100 to 106; the 12 px cell rows 97 to 108, ink from its second.
        let (top, bottom, _, _) = ink(&render(|canvas| {
            canvas.draw_str(10, 100, "国".as_bytes(), 1, 0xffff, 100);
        }));
        assert!(top >= 98 && (106..=108).contains(&bottom), "{top}..{bottom}");
        // Scale 2: still 12 px, centred on the 14 px ASCII.
        let (top, bottom, left, right) = ink(&render(|canvas| {
            canvas.draw_str(10, 100, "国".as_bytes(), 2, 0xffff, 100);
        }));
        assert!(top >= 102 && bottom <= 112 && left >= 10 && right <= 20, "{top}..{bottom} {left}..{right}");
    }

    #[test]
    fn mixed_text_advances_by_each_font_and_stops_at_the_width() {
        let whole = render(|canvas| assert_eq!(canvas.draw_str(0, 50, "A你B".as_bytes(), 1, 0xffff, 100), 6 + 12 + 5));
        assert_eq!(ink(&whole).3, 22);
        // Whole characters only: 你 doesn't fit in 15 px after A.
        let cut = render(|canvas| assert_eq!(canvas.draw_str(0, 50, "A你B".as_bytes(), 1, 0xffff, 15), 5));
        assert_eq!(ink(&cut).3, 4);
    }

    #[test]
    fn a_character_the_font_lacks_is_a_box() {
        let lacking = render(|canvas| {
            canvas.draw_str(0, 50, "\u{1F600}".as_bytes(), 1, 0xffff, 100);
        });
        assert_eq!(ink(&lacking), (48, 58, 0, 10));
    }
}
