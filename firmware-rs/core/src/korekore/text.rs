//! Type on the korekore screens: DotGothic16 at the design's sizes, and the "korekore" wordmark, as 4-bit coverage
//! from `tools/korekore-box-fonts.py` (glyphs.rs), blended over what is already drawn. Characters a size doesn't
//! have, such as Chinese in a task title, fall back to the 12 px table the robot screens use (font.rs).

use crate::canvas::Canvas;
use crate::font;

pub struct Glyph {
    pub code: u32,
    pub advance: u8,
    /// Where the coverage starts, from the pen position and from the top of the line.
    pub left: i8,
    pub top: i8,
    pub width: u8,
    pub height: u8,
    /// The first byte of its coverage in the face's `alpha`.
    pub at: u32,
}

pub struct Face {
    pub size: u8,
    pub ascent: u8,
    pub descent: u8,
    /// Sorted by code.
    pub glyphs: &'static [Glyph],
    pub alpha: &'static [u8],
}

pub struct Bitmap {
    pub width: u16,
    pub height: u16,
    pub alpha: &'static [u8],
}

fn nibble(alpha: &[u8], index: usize) -> u8 {
    let byte = alpha[index / 2];
    if index.is_multiple_of(2) { byte & 15 } else { byte >> 4 }
}

impl Face {
    fn find(&self, character: char) -> Option<&Glyph> {
        let code = character as u32;
        self.glyphs.binary_search_by_key(&code, |glyph| glyph.code).ok().map(|index| &self.glyphs[index])
    }

    /// The line's height: from the top of the line to the bottom of the descenders.
    pub fn line(&self) -> i32 {
        i32::from(self.ascent) + i32::from(self.descent)
    }
}

/// How one character is drawn at a face.
enum Piece<'a> {
    Glyph(&'a Glyph),
    Wide(usize),
    Missing,
}

fn piece(face: &Face, character: char) -> Piece<'_> {
    if let Some(glyph) = face.find(character) {
        return Piece::Glyph(glyph);
    }
    match font::lookup(character) {
        Some(index) => Piece::Wide(index),
        None => Piece::Missing,
    }
}

fn advance(face: &Face, piece: &Piece) -> i32 {
    match piece {
        Piece::Glyph(glyph) => i32::from(glyph.advance),
        Piece::Wide(_) => font::CELL + 1,
        Piece::Missing => i32::from(face.size) / 2,
    }
}

/// The characters of UTF-8 text up to a NUL or the first byte that isn't valid.
fn characters(text: &[u8]) -> core::str::Chars<'_> {
    let text = text.split(|&byte| byte == 0).next().unwrap_or_default();
    let valid = match core::str::from_utf8(text) {
        Ok(valid) => valid,
        Err(error) => core::str::from_utf8(&text[..error.valid_up_to()]).unwrap_or_default(),
    };
    valid.chars()
}

/// How wide `text` is at `face`.
pub fn width(face: &Face, text: &[u8]) -> i32 {
    characters(text).map(|character| advance(face, &piece(face, character))).sum()
}

/// Draws `text` with the top of its line at `y`, as many whole characters as fit in `max_width`. Returns the width
/// drawn.
pub fn draw(canvas: &mut Canvas, face: &Face, x: i32, y: i32, text: &[u8], color: u16, max_width: i32) -> i32 {
    let mut pen = x;
    for character in characters(text) {
        let piece = piece(face, character);
        let step = advance(face, &piece);
        if pen + step - x > max_width {
            break;
        }
        match piece {
            Piece::Glyph(glyph) => {
                let (left, top) = (pen + i32::from(glyph.left), y + i32::from(glyph.top));
                let width = usize::from(glyph.width);
                for row in 0..usize::from(glyph.height) {
                    for column in 0..width {
                        let alpha = nibble(face.alpha, glyph.at as usize * 2 + row * width + column);
                        canvas.blend(left + column as i32, top + row as i32, color, alpha);
                    }
                }
            }
            // The 12 px table, centred on the line.
            Piece::Wide(index) => {
                let top = y + (face.line() - font::CELL) / 2;
                for row in 0..font::CELL {
                    let bits = font::row(index, row as usize);
                    for column in 0..font::CELL {
                        if bits & (0x800 >> column) != 0 {
                            canvas.fill_rect(pen + column, top + row, 1, 1, color);
                        }
                    }
                }
            }
            Piece::Missing => {}
        }
        pen += step;
    }
    pen - x
}

/// `draw` centred on `center_x`.
pub fn draw_centered(canvas: &mut Canvas, face: &Face, center_x: i32, y: i32, text: &[u8], color: u16) {
    draw(canvas, face, center_x - width(face, text) / 2, y, text, color, i32::MAX);
}

/// `draw` ending at `right`.
pub fn draw_right(canvas: &mut Canvas, face: &Face, right: i32, y: i32, text: &[u8], color: u16) {
    draw(canvas, face, right - width(face, text), y, text, color, i32::MAX);
}

/// A coverage bitmap with its top left at (x, y).
pub fn draw_bitmap(canvas: &mut Canvas, bitmap: &Bitmap, x: i32, y: i32, color: u16) {
    let width = usize::from(bitmap.width);
    for row in 0..usize::from(bitmap.height) {
        for column in 0..width {
            canvas.blend(x + column as i32, y + row as i32, color, nibble(bitmap.alpha, row * width + column));
        }
    }
}
