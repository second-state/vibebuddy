//! The korekore look on the box (ADR-0020, ADR-0021): its colours and pixel art, generated from docs/design/korekore
//! by `tools/korekore-codegen.py`; its type, from `tools/korekore-box-fonts.py`; and the screens drawn with them
//! (display.rs draws them in place of the robot screens when the box is told to).

pub mod glyphs;
pub mod sprites;
pub mod text;
pub mod theme;

/// A palette-indexed sprite, kept in flash: one index per pixel, row by row; 0 is transparent and n is
/// `palette[n - 1]`, in RGB565.
pub struct Sprite {
    pub width: u16,
    pub height: u16,
    pub palette: &'static [u16],
    pub pixels: &'static [u8],
}

impl Sprite {
    /// The colour at (`x`, `y`), or None where the sprite is transparent or outside it.
    pub fn pixel(&self, x: u16, y: u16) -> Option<u16> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let index = self.pixels[usize::from(y) * usize::from(self.width) + usize::from(x)];
        index.checked_sub(1).map(|n| self.palette[usize::from(n)])
    }
}
