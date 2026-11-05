//! The korekore UI (ADR-0020, ADR-0021): the app's only UI. Its theme, pixel art and fonts are generated from
//! docs/design/korekore by `tools/korekore-codegen.py` and `tools/korekore-fonts.py`.

// The generated theme and sprites hold all the design's tokens, and the views do not use each one.
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

/// The bundled fonts (fonts/korekore), registered with iced at start.
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

/// The app's icon, as the Mac app shows it in the Dock (`KorekoreAppIcon`): Kore on a `sun` tile with an ink outline,
/// on macOS's icon grid (a 824 px tile on a 1024 px canvas, corner radius 185). RGBA, `size` × `size`, each pixel
/// the average of 8 × 8 samples, so the tile's edge is smooth and Kore stays sharp at whole sizes.
pub fn app_icon(size: u32) -> Vec<u8> {
    const SAMPLES: u32 = 8;
    let geometry = AppIconGeometry::new();
    let unit = 1024.0 / f64::from(size);
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            // Premultiplied sums, so a half-covered edge pixel keeps its colour.
            let mut sum = [0.0f64; 4];
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let px = (f64::from(x) + (f64::from(sx) + 0.5) / f64::from(SAMPLES)) * unit;
                    let py = (f64::from(y) + (f64::from(sy) + 0.5) / f64::from(SAMPLES)) * unit;
                    if let Some(rgb) = geometry.colour_at(px, py) {
                        sum[0] += f64::from(rgb >> 16 & 0xFF);
                        sum[1] += f64::from(rgb >> 8 & 0xFF);
                        sum[2] += f64::from(rgb & 0xFF);
                        sum[3] += 1.0;
                    }
                }
            }
            let covered = sum[3];
            let alpha = covered / f64::from(SAMPLES * SAMPLES);
            let channel = |value: f64| if covered > 0.0 { (value / covered).round() as u8 } else { 0 };
            out.extend_from_slice(&[channel(sum[0]), channel(sum[1]), channel(sum[2]), (alpha * 255.0).round() as u8]);
        }
    }
    out
}

/// Where the app icon's parts sit on the 1024 px canvas.
struct AppIconGeometry {
    /// Half the side of the path the outline is drawn along: the tile, inset by half the outline.
    half: f64,
    stroke: f64,
    kore_scale: u32,
    kore_left: u32,
    kore_top: u32,
}

impl AppIconGeometry {
    const CANVAS: f64 = 1024.0;
    const TILE: f64 = 824.0;
    const RADIUS: f64 = 185.0;

    fn new() -> Self {
        let kore = &sprites::KORE_NORMAL;
        // The badge's 2 pt outline on its 46 pt height, scaled to the tile; Kore at a whole-pixel scale, about 70 %
        // of the tile wide. Both as on the Mac.
        let stroke = (Self::TILE * f64::from(theme::STROKE_BASE) / 46.0).round();
        let kore_scale = (Self::TILE * 0.7) as u32 / kore.width;
        let canvas = Self::CANVAS as u32;
        Self {
            half: (Self::TILE - stroke) / 2.0,
            stroke,
            kore_scale,
            kore_left: (canvas - kore.width * kore_scale) / 2,
            kore_top: (canvas - kore.height * kore_scale) / 2,
        }
    }

    /// The colour at a point of the canvas (0xRRGGBB), or none where it is transparent.
    fn colour_at(&self, x: f64, y: f64) -> Option<u32> {
        let kore = &sprites::KORE_NORMAL;
        let (kx, ky) = (x - f64::from(self.kore_left), y - f64::from(self.kore_top));
        if kx >= 0.0 && ky >= 0.0 {
            let (col, row) = (kx as u32 / self.kore_scale, ky as u32 / self.kore_scale);
            if col < kore.width
                && row < kore.height
                && let Some(index) = kore.pixels[(row * kore.width + col) as usize].checked_sub(1)
            {
                return Some(kore.palette[usize::from(index)]);
            }
        }
        // Signed distance to the rounded square the outline follows: negative inside.
        let centre = Self::CANVAS / 2.0;
        let (dx, dy) = ((x - centre).abs() - (self.half - Self::RADIUS), (y - centre).abs() - (self.half - Self::RADIUS));
        let outside = dx.max(0.0).hypot(dy.max(0.0));
        let distance = outside + dx.max(dy).min(0.0) - Self::RADIUS;
        if distance.abs() <= self.stroke / 2.0 {
            Some(0x1C1730)
        } else if distance < 0.0 {
            Some(0xFFD23F)
        } else {
            None
        }
    }
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

    /// The tab this section stands for: same state, same messages.
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

    /// A section still being tried out: the sidebar marks it "Beta".
    pub fn beta(self) -> bool {
        self == Section::Conversation
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
    fn the_app_icon_is_kore_on_a_sun_tile_with_an_ink_outline() {
        let size = 64;
        let icon = app_icon(size);
        assert_eq!(icon.len(), (size * size * 4) as usize);
        let at = |x: u32, y: u32| &icon[((y * size + x) * 4) as usize..][..4];
        // Outside the tile, in its sun fill (between the outline and Kore), on the outline at the middle of the
        // left edge, and on Kore's screen.
        assert_eq!(at(1, 1), [0; 4]);
        assert_eq!(at(12, 32), [0xFF, 0xD2, 0x3F, 0xFF]);
        assert_eq!(at(7, 32)[..3], [0x1C, 0x17, 0x30]);
        assert_eq!(at(30, 25), [0x3B, 0x2F, 0x63, 0xFF]);
    }

    /// Writes the app's icon files from `app_icon`, so they always match it: the Windows `.ico` and the Linux `.svg`.
    /// Run it by hand after changing Kore or the badge: `cargo test -p vibebuddy-desktop write_app_icons -- --ignored`.
    #[test]
    #[ignore]
    fn write_app_icons() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        std::fs::write(root.join("packaging/windows/vibebuddy.ico"), app_icon_ico()).unwrap();
        std::fs::write(root.join("packaging/linux/vibebuddy.svg"), app_icon_svg()).unwrap();
    }

    /// An `.ico` of PNG images, which Windows reads at every size since Vista.
    fn app_icon_ico() -> Vec<u8> {
        use ::image::ImageEncoder;
        let sizes = [16u32, 20, 24, 32, 40, 48, 64, 128, 256];
        let images: Vec<Vec<u8>> = sizes
            .iter()
            .map(|&size| {
                let mut png = Vec::new();
                ::image::codecs::png::PngEncoder::new(&mut png)
                    .write_image(&app_icon(size), size, size, ::image::ExtendedColorType::Rgba8)
                    .unwrap();
                png
            })
            .collect();
        let mut ico = vec![0, 0, 1, 0];
        ico.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
        let mut offset = 6 + 16 * sizes.len() as u32;
        for (size, png) in sizes.iter().zip(&images) {
            // A side of 256 is written as 0.
            let side = (*size % 256) as u8;
            ico.extend_from_slice(&[side, side, 0, 0, 1, 0, 32, 0]);
            ico.extend_from_slice(&(png.len() as u32).to_le_bytes());
            ico.extend_from_slice(&offset.to_le_bytes());
            offset += png.len() as u32;
        }
        images.iter().for_each(|png| ico.extend_from_slice(png));
        ico
    }

    /// The same badge as vectors: the outlined tile, then Kore's pixels as runs of rects.
    fn app_icon_svg() -> String {
        let geometry = AppIconGeometry::new();
        let kore = &sprites::KORE_NORMAL;
        let corner = AppIconGeometry::CANVAS / 2.0 - geometry.half;
        let mut svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 1024 1024\">\n  \
             <rect x=\"{corner}\" y=\"{corner}\" width=\"{side}\" height=\"{side}\" rx=\"{radius}\" \
             fill=\"#FFD23F\" stroke=\"#1C1730\" stroke-width=\"{stroke}\"/>\n  <g shape-rendering=\"crispEdges\">\n",
            side = geometry.half * 2.0,
            radius = AppIconGeometry::RADIUS,
            stroke = geometry.stroke,
        );
        let scale = geometry.kore_scale;
        for row in 0..kore.height {
            let mut col = 0;
            while col < kore.width {
                let index = kore.pixels[(row * kore.width + col) as usize];
                let start = col;
                while col < kore.width && kore.pixels[(row * kore.width + col) as usize] == index {
                    col += 1;
                }
                if let Some(n) = index.checked_sub(1) {
                    svg += &format!(
                        "    <rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{scale}\" fill=\"#{:06X}\"/>\n",
                        geometry.kore_left + start * scale,
                        geometry.kore_top + row * scale,
                        (col - start) * scale,
                        kore.palette[usize::from(n)],
                    );
                }
            }
        }
        svg + "  </g>\n</svg>\n"
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
