//! The korekore UI (ADR-0020, ADR-0021): for now its theme, pixel art and fonts, generated from docs/design/korekore
//! by `tools/korekore-codegen.py` and `tools/korekore-fonts.py`. It opens only with `VIBEBUDDY_UI=korekore`
//! while it is being built; the current UI stays what everyone else gets.

// The views that use all this come in the next steps.
#![allow(dead_code)]

mod account;
mod controls;
mod guide;
pub(crate) mod pages;
pub mod picture;
#[cfg(test)]
mod snapshots;
pub mod sprites;
pub mod theme;
pub mod view;

use iced::widget::image;

use crate::i18n::tr;

/// Whether this launch asked for the korekore UI. Read once at launch.
pub fn requested() -> bool {
    std::env::var("VIBEBUDDY_UI").is_ok_and(|ui| ui == "korekore")
}

/// The bundled fonts (fonts/korekore), registered with iced at start only when the korekore UI is on.
pub const FONTS: [&[u8]; 5] = [
    include_bytes!("../../../fonts/korekore/MPLUSRounded1c-Regular.ttf"),
    include_bytes!("../../../fonts/korekore/MPLUSRounded1c-Bold.ttf"),
    include_bytes!("../../../fonts/korekore/MPLUSRounded1c-ExtraBold.ttf"),
    include_bytes!("../../../fonts/korekore/DotGothic16-Regular.ttf"),
    include_bytes!("../../../fonts/korekore/FusionPixel12-Regular.ttf"),
];

/// Text that names no font: M PLUS Rounded 1c, regular.
pub const DEFAULT_FONT: iced::Font = theme::typography::BODY.font;

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
        rgba(self.width, self.height, self.palette, self.pixels, scale, tint)
    }

    /// The sprite in gray, as CSS `grayscale(1) brightness(b)` draws it: each colour's luminance (Rec. 709
    /// weights), scaled by `brightness`. The canvas grays Kore this way when the box has no link.
    pub fn grayed(&self, brightness: f32) -> GraySprite {
        let palette = self
            .palette
            .iter()
            .map(|&rgb| {
                let (r, g, b) = ((rgb >> 16 & 0xFF) as f32, (rgb >> 8 & 0xFF) as f32, (rgb & 0xFF) as f32);
                let level = ((0.2126 * r + 0.7152 * g + 0.0722 * b) * brightness).round().clamp(0.0, 255.0) as u32;
                level << 16 | level << 8 | level
            })
            .collect();
        GraySprite { width: self.width, height: self.height, palette, pixels: self.pixels }
    }

    /// An image of the sprite at `scale` whole times its size; show it at that size, so nothing resamples it.
    pub fn handle(&self, scale: u32, tint: Option<u32>) -> image::Handle {
        image::Handle::from_rgba(self.width * scale, self.height * scale, self.rgba(scale, tint))
    }
}

/// A sprite with its palette grayed; drawn like any other.
pub struct GraySprite {
    width: u32,
    height: u32,
    palette: Vec<u32>,
    pixels: &'static [u8],
}

impl GraySprite {
    pub fn rgba(&self, scale: u32) -> Vec<u8> {
        rgba(self.width, self.height, &self.palette, self.pixels, scale, None)
    }

    pub fn handle(&self, scale: u32, tint: Option<u32>) -> image::Handle {
        let bytes = rgba(self.width, self.height, &self.palette, self.pixels, scale, tint);
        image::Handle::from_rgba(self.width * scale, self.height * scale, bytes)
    }
}

fn rgba(width: u32, height: u32, palette: &[u32], pixels: &[u8], scale: u32, tint: Option<u32>) -> Vec<u8> {
    let (out_width, out_height) = (width * scale, height * scale);
    let mut bytes = Vec::with_capacity((out_width * out_height * 4) as usize);
    for y in 0..out_height {
        for x in 0..out_width {
            let index = pixels[((y / scale) * width + x / scale) as usize];
            bytes.extend_from_slice(&match index.checked_sub(1) {
                None => [0; 4],
                Some(n) => {
                    let rgb = tint.unwrap_or(palette[usize::from(n)]);
                    [(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, 0xFF]
                }
            });
        }
    }
    bytes
}

/// The tray's Kore (canvas: "Mac · Menu bar", bottom row): in colour online, with a `sun` dot beside it when an
/// agent needs you, gray and asleep without a link. ARGB32 rows, as StatusNotifierItem pixmaps are.
pub fn tray_icon(picture: &picture::BoxPicture, scale: u32) -> (u32, u32, Vec<u8>) {
    // The dot is the design's 7 px circle with its 1.5 px ink edge, at whole pixels.
    const DOT: [&str; 7] = ["..iii..", ".isssi.", "isssssi", "isssssi", "isssssi", ".isssi.", "..iii.."];
    let needs_you = picture.needs_you();
    let kore = if needs_you { &sprites::KORE_INPUT } else { &sprites::KORE_NORMAL };
    let rgba = if picture.mood == picture::Mood::NoLink {
        sprites::KORE_SLEEP.grayed(0.75).rgba(scale)
    } else {
        kore.rgba(scale, None)
    };
    let kore_width = kore.width * scale;
    let width = if needs_you { kore_width + (4 + 7) * scale } else { kore_width };
    let height = kore.height * scale;
    let mut out = vec![0u8; (width * height * 4) as usize];
    for y in 0..height {
        for x in 0..width {
            let pixel = if x < kore_width {
                let at = ((y * kore_width + x) * 4) as usize;
                [rgba[at + 3], rgba[at], rgba[at + 1], rgba[at + 2]]
            } else {
                let (dx, dy) = ((x - kore_width) / scale, y / scale);
                let row = dy.checked_sub((kore.height - 7) / 2).and_then(|row| DOT.get(row as usize));
                match row.and_then(|row| dx.checked_sub(4).and_then(|col| row.as_bytes().get(col as usize))) {
                    Some(b'i') => [0xFF, 0x1C, 0x17, 0x30],
                    Some(b's') => [0xFF, 0xFF, 0xD2, 0x3F],
                    _ => [0; 4],
                }
            };
            out[((y * width + x) * 4) as usize..][..4].copy_from_slice(&pixel);
        }
    }
    (width, height, out)
}

/// The settings window's sections, in the sidebar's order (docs/design/korekore/README.md).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Character,
    Agents,
    Conversation,
    Device,
    General,
    Advanced,
}

impl Section {
    pub const ALL: [Section; 6] =
        [Section::Character, Section::Agents, Section::Conversation, Section::Device, Section::General, Section::Advanced];

    pub fn title(self) -> String {
        match self {
            Section::Character => tr("Character", &[]),
            Section::Agents => tr("Agents", &[]),
            Section::Conversation => tr("Conversation", &[]),
            Section::Device => tr("Device", &[]),
            Section::General => tr("General", &[]),
            Section::Advanced => tr("Advanced", &[]),
        }
    }

    pub fn subtitle(self) -> String {
        match self {
            Section::Character => tr("Who lives in the box", &[]),
            Section::Agents => tr("Who the box watches", &[]),
            Section::Conversation => tr("Hold K0 on the box and ask", &[]),
            Section::Device => tr("The box, its firmware and its network", &[]),
            Section::General => tr("How korekore behaves on this computer", &[]),
            Section::Advanced => tr("For when something's off", &[]),
        }
    }

    /// The current UI's tab this section stands for: same state, same messages.
    pub fn tab(self) -> crate::Tab {
        match self {
            Section::Character => crate::Tab::Sound,
            Section::Agents => crate::Tab::Agents,
            Section::Conversation => crate::Tab::Conversation,
            Section::Device => crate::Tab::Device,
            Section::General => crate::Tab::General,
            Section::Advanced => crate::Tab::Advanced,
        }
    }

    pub fn of(tab: crate::Tab) -> Self {
        Section::ALL.into_iter().find(|section| section.tab() == tab).unwrap_or(Section::Character)
    }

    pub fn tile(self) -> &'static str {
        match self {
            Section::Character => "icon-nav-char",
            Section::Agents => "icon-nav-agents",
            Section::Conversation => "icon-nav-chat",
            Section::Device => "icon-nav-box",
            Section::General => "icon-nav-gen",
            Section::Advanced => "icon-nav-adv",
        }
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
    fn the_tray_icon_adds_a_sun_dot_only_when_an_agent_needs_you() {
        use crate::status::{Activity, Status};
        let mut status = Status::default();
        status.device.connected = true;
        let (width, height, pixels) = tray_icon(&picture::BoxPicture::derive(Some(&status)), 2);
        assert_eq!((width, height, pixels.len()), (40, 36, 40 * 36 * 4));
        status.activities = vec![Activity { agent: "codex".into(), title: "CX:Ask".into(), state: "needs_input".into(), ..Activity::default() }];
        let (width, _, pixels) = tray_icon(&picture::BoxPicture::derive(Some(&status)), 2);
        assert_eq!(width, 62);
        // The dot's centre, in ARGB: sun.
        let at = ((17 * 62 + 40 + 2 * 7) * 4) as usize;
        assert_eq!(&pixels[at..at + 4], [0xFF, 0xFF, 0xD2, 0x3F]);
    }

    #[test]
    fn the_theme_matches_the_tokens() {
        assert_eq!(theme::INK, iced::Color::from_rgb8(0x1C, 0x17, 0x30));
        assert_eq!(theme::BOX_GROUND, theme::INK);
        assert_eq!(theme::typography::TITLE.size, 28.0);
        assert_eq!(theme::STICKER_SM, 4.0);
    }
}
