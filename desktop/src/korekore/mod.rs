//! The korekore UI (ADR-0020, ADR-0021): for now its theme, pixel art and fonts, generated from docs/design/korekore
//! by `tools/korekore-codegen.py` and `tools/korekore-fonts.py`. It opens only with `VIBEBUDDY_UI=korekore`
//! while it is being built; the current UI stays what everyone else gets.

// The views that use all this come in the next steps.
#![allow(dead_code)]

pub mod sprites;
pub mod theme;

use iced::Task;
use iced::widget::image;

/// Whether this launch asked for the korekore UI. Read once at launch.
pub fn requested() -> bool {
    std::env::var("VIBEBUDDY_UI").is_ok_and(|ui| ui == "korekore")
}

/// The bundled fonts (fonts/korekore), registered with iced only when the korekore UI is on.
const FONTS: [&[u8]; 4] = [
    include_bytes!("../../../fonts/korekore/MPLUSRounded1c-Regular.ttf"),
    include_bytes!("../../../fonts/korekore/MPLUSRounded1c-Bold.ttf"),
    include_bytes!("../../../fonts/korekore/MPLUSRounded1c-ExtraBold.ttf"),
    include_bytes!("../../../fonts/korekore/DotGothic16-Regular.ttf"),
];

pub fn load_fonts<T: Send + 'static>() -> Task<T> {
    Task::batch(FONTS.map(|font| iced::font::load(font).discard()))
}

/// A palette-indexed sprite: one index per pixel, row by row; 0 is transparent and n is `palette[n - 1]`,
/// as 0xRRGGBB.
pub struct Sprite {
    pub width: u32,
    pub height: u32,
    pub palette: &'static [u32],
    pub pixels: &'static [u8],
}

impl Sprite {
    /// RGBA bytes at `scale` whole times the size, nearest-neighbour. `tint` (0xRRGGBB) replaces every colour,
    /// for the one-colour tag.
    pub fn rgba(&self, scale: u32, tint: Option<u32>) -> Vec<u8> {
        let (width, height) = (self.width * scale, self.height * scale);
        let mut bytes = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                let index = self.pixels[((y / scale) * self.width + x / scale) as usize];
                bytes.extend_from_slice(&match index.checked_sub(1) {
                    None => [0; 4],
                    Some(n) => {
                        let rgb = tint.unwrap_or(self.palette[usize::from(n)]);
                        [(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, 0xFF]
                    }
                });
            }
        }
        bytes
    }

    /// An image of the sprite at `scale` whole times its size; show it at that size, so nothing resamples it.
    pub fn handle(&self, scale: u32, tint: Option<u32>) -> image::Handle {
        image::Handle::from_rgba(self.width * scale, self.height * scale, self.rgba(scale, tint))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaling_repeats_each_pixel_and_keeps_transparency() {
        let kore = &sprites::KORE_NORMAL;
        let rgba = kore.rgba(3, None);
        assert_eq!(rgba.len(), (20 * 3 * 18 * 3 * 4) as usize);
        let at = |x: u32, y: u32| &rgba[((y * 60 + x) * 4) as usize..][..4];
        // Pixel (4, 0) is the pink key: it covers x 12..15 and y 0..3.
        for (x, y) in [(12, 0), (14, 2)] {
            assert_eq!(at(x, y), [0xF0, 0x58, 0x7F, 0xFF]);
        }
        assert_eq!(at(0, 0), [0; 4]);
    }

    #[test]
    fn a_tint_recolours_only_what_is_drawn() {
        let tag = &sprites::TAG_YOUR_VIBEBUDDY;
        let rgba = tag.rgba(1, Some(0xE2466F));
        assert!(rgba.chunks(4).all(|pixel| pixel == [0; 4] || pixel == [0xE2, 0x46, 0x6F, 0xFF]));
        assert!(rgba.chunks(4).any(|pixel| pixel[3] == 0xFF));
    }

    #[test]
    fn every_sprite_is_whole_and_indexes_its_palette() {
        for (name, sprite) in sprites::ALL {
            assert_eq!(sprite.pixels.len(), (sprite.width * sprite.height) as usize, "{name}");
            assert!(sprite.pixels.iter().all(|&index| usize::from(index) <= sprite.palette.len()), "{name}");
        }
    }

    #[test]
    fn the_theme_matches_the_tokens() {
        assert_eq!(theme::INK, iced::Color::from_rgb8(0x1C, 0x17, 0x30));
        assert_eq!(theme::BOX_GROUND, theme::INK);
        assert_eq!(theme::typography::TITLE.size, 28.0);
        assert_eq!(theme::STICKER_SM, 4.0);
    }
}
