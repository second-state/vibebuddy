//! The korekore look's colours and pixel art (ADR-0020, ADR-0021), generated from docs/design/korekore by
//! `tools/korekore-codegen.py`. Nothing draws with them yet: the korekore screens come later, and until then the
//! linker leaves all of this out of the device image.

pub mod sprites;
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
