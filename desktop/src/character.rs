//! Character packs as the app puts them together, the Mac app's `Look.swift` in Rust: reading a pack's
//! pools and look, laying out a pack exactly as `tools/character_pack.py` does, swapping in the lines
//! of a form of address, and turning the user's drawings into a look (firmware-rs/core/src/look.rs).
//! The app never draws or synthesizes anything (ADR-0009); it only rearranges what it is given.

const HEADER_BYTES: usize = 1024;
const LOOK_WIDTH: usize = 48;
const LOOK_HEIGHT: usize = 64;
const LOOK_COLORS: usize = 16;
const LOOK_FRAME_BYTES: usize = LOOK_WIDTH * LOOK_HEIGHT / 2;
const LOOK_BYTES: usize = 8 + LOOK_COLORS * 2 + 4 * LOOK_FRAME_BYTES;
const FIGURE_HEIGHT: usize = 60;

fn u16_at(data: &[u8], at: usize) -> usize {
    u16::from_le_bytes([data[at], data[at + 1]]) as usize
}

fn u32_at(data: &[u8], at: usize) -> usize {
    u32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]]) as usize
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { 0xEDB8_8320 ^ (crc >> 1) } else { crc >> 1 };
        }
    }
    !crc
}

/// One line: its ADPCM bytes and how many samples they hold.
pub type Line = (Vec<u8>, u32);

/// A Character pack taken apart: everything needed to put it back together.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pack {
    pub id: String,
    pub pools: Vec<Vec<Line>>,
    pub look: Option<Vec<u8>>,
}

impl Pack {
    /// Reads a Character pack; None for anything else, an old voice pack included.
    pub fn parse(data: &[u8]) -> Option<Pack> {
        if data.len() <= HEADER_BYTES || &data[..4] != b"VBCP" || data[52] != 1 {
            return None;
        }
        let id_bytes: Vec<u8> = data[16..48].iter().copied().take_while(|&byte| byte != 0).collect();
        let id = String::from_utf8(id_bytes).ok().filter(|id| !id.is_empty())?;
        let mut pools = Vec::new();
        for occasion in 0..data[53] as usize {
            let first = u16_at(data, 56 + occasion * 4);
            let count = u16_at(data, 58 + occasion * 4);
            let mut lines = Vec::new();
            for line in first..first + count {
                let offset = u32_at(data, 128 + line * 8);
                let samples = u32_at(data, 132 + line * 8);
                lines.push((data.get(offset..offset + samples.div_ceil(2))?.to_vec(), samples as u32));
            }
            pools.push(lines);
        }
        let look = (u32_at(data, 4) == 2 && u32_at(data, 1012) > 0)
            .then(|| {
                let offset = u32_at(data, 1008);
                data.get(offset..offset + u32_at(data, 1012)).map(<[u8]>::to_vec)
            })
            .flatten();
        Some(Pack { id, pools, look })
    }

    /// Lays the pack out byte for byte as tools/character_pack.py does; None if it doesn't fit the header.
    pub fn build(&self) -> Option<Vec<u8>> {
        let id = self.id.as_bytes();
        let lines: usize = self.pools.iter().map(Vec::len).sum();
        let limit = if self.look.is_some() { (1008 - 128) / 8 } else { (1020 - 128) / 8 };
        if id.is_empty() || id.len() > 31 || lines > limit || self.pools.len() > (128 - 56) / 4 {
            return None;
        }
        let mut head = vec![0u8; HEADER_BYTES];
        let mut payload = Vec::new();
        let mut line = 0;
        for (occasion, pool) in self.pools.iter().enumerate() {
            let first = if pool.is_empty() { 0 } else { line };
            head[56 + occasion * 4..58 + occasion * 4].copy_from_slice(&(first as u16).to_le_bytes());
            head[58 + occasion * 4..60 + occasion * 4].copy_from_slice(&(pool.len() as u16).to_le_bytes());
            for (audio, samples) in pool {
                head[128 + line * 8..132 + line * 8].copy_from_slice(&((HEADER_BYTES + payload.len()) as u32).to_le_bytes());
                head[132 + line * 8..136 + line * 8].copy_from_slice(&samples.to_le_bytes());
                payload.extend_from_slice(audio);
                line += 1;
            }
        }
        if let Some(look) = &self.look {
            payload.resize(payload.len().next_multiple_of(4), 0);
            head[1008..1012].copy_from_slice(&((HEADER_BYTES + payload.len()) as u32).to_le_bytes());
            head[1012..1016].copy_from_slice(&(look.len() as u32).to_le_bytes());
            payload.extend_from_slice(look);
        }
        head[..4].copy_from_slice(b"VBCP");
        head[4..8].copy_from_slice(&(if self.look.is_some() { 2u32 } else { 1 }).to_le_bytes());
        head[8..12].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        head[12..16].copy_from_slice(&crc32(&payload).to_le_bytes());
        head[16..16 + id.len()].copy_from_slice(id);
        head[48..52].copy_from_slice(&16000u32.to_le_bytes());
        head[52] = 1;
        head[53] = self.pools.len() as u8;
        head[54..56].copy_from_slice(&(lines as u16).to_le_bytes());
        let header_crc = crc32(&head[..1020]);
        head[1020..1024].copy_from_slice(&header_crc.to_le_bytes());
        head.extend_from_slice(&payload);
        Some(head)
    }

    /// The same Character with a form of address: `variant` (characters/<id>/address/<form>.bin) holds
    /// the lines said with it, which take the place of the first lines of each pool.
    pub fn with_address(mut self, variant: &Pack) -> Pack {
        for (pool, lines) in self.pools.iter_mut().zip(&variant.pools) {
            if lines.len() <= pool.len() {
                pool.splice(..lines.len(), lines.iter().cloned());
            }
        }
        self
    }

    /// A new Character of this one's voice and lines and a look the user brought.
    pub fn with_look(mut self, look: Vec<u8>, id: &str) -> Pack {
        self.look = Some(look);
        self.id = id.to_owned();
        self
    }
}

/// An image as plain pixels: RGBA, 8 bits each, row by row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

impl Image {
    fn rgb(&self, x: usize, y: usize) -> [u8; 3] {
        let at = (y * self.width + x) * 4;
        [self.pixels[at], self.pixels[at + 1], self.pixels[at + 2]]
    }
}

/// Opaque means alpha of at least 128 in an image that has transparency. An image without any (a white or
/// magenta background) has its background flood-filled away from the edges, so white inside the figure stays.
fn opaque_mask(image: &Image) -> Vec<bool> {
    let count = image.width * image.height;
    let mut opaque: Vec<bool> = (0..count).map(|index| image.pixels[index * 4 + 3] >= 128).collect();
    if opaque.iter().any(|&pixel| !pixel) {
        return opaque;
    }
    let background = |index: usize| {
        let [r, g, b] = [image.pixels[index * 4], image.pixels[index * 4 + 1], image.pixels[index * 4 + 2]];
        r.min(g).min(b) > 225 || (r > 230 && g < 30 && b > 230)
    };
    let (w, h) = (image.width, image.height);
    let mut stack: Vec<usize> = (0..w).flat_map(|x| [x, (h - 1) * w + x]).chain((0..h).flat_map(|y| [y * w, y * w + w - 1])).collect();
    while let Some(index) = stack.pop() {
        if !opaque[index] || !background(index) {
            continue;
        }
        opaque[index] = false;
        let (x, y) = (index % w, index / w);
        if x > 0 {
            stack.push(index - 1);
        }
        if x + 1 < w {
            stack.push(index + 1);
        }
        if y > 0 {
            stack.push(index - w);
        }
        if y + 1 < h {
            stack.push(index + w);
        }
    }
    opaque
}

/// The figure's bounding box: left, top, right, bottom (exclusive).
fn bounding_box(image: &Image, opaque: &[bool]) -> Option<(usize, usize, usize, usize)> {
    let mut found: Option<(usize, usize, usize, usize)> = None;
    for y in 0..image.height {
        for x in 0..image.width {
            if opaque[y * image.width + x] {
                let (l, t, r, b) = found.unwrap_or((x, y, x + 1, y + 1));
                found = Some((l.min(x), t.min(y), r.max(x + 1), b.max(y + 1)));
            }
        }
    }
    found
}

/// Shrinks the figure into a cell: each cell pixel averages the opaque source pixels under it, and is
/// kept only where the figure covers most of it, so there is no halo.
fn cell(image: &Image, opaque: &[bool], (left, top, right, bottom): (usize, usize, usize, usize), scale: f64) -> Vec<Option<[u8; 3]>> {
    let (source_w, source_h) = (right - left, bottom - top);
    let w = ((source_w as f64 * scale).round() as usize).clamp(1, LOOK_WIDTH);
    let h = ((source_h as f64 * scale).round() as usize).clamp(1, LOOK_HEIGHT);
    let mut out = vec![None; LOOK_WIDTH * LOOK_HEIGHT];
    let (cell_left, cell_top) = ((LOOK_WIDTH - w) / 2, LOOK_HEIGHT - h);
    for cy in 0..h {
        for cx in 0..w {
            let x0 = left + cx * source_w / w;
            let x1 = (left + (cx + 1) * source_w / w).max(x0 + 1);
            let y0 = top + cy * source_h / h;
            let y1 = (top + (cy + 1) * source_h / h).max(y0 + 1);
            let (mut sum, mut covered) = ([0usize; 3], 0);
            for y in y0..y1 {
                for x in x0..x1 {
                    if opaque[y * image.width + x] {
                        let rgb = image.rgb(x, y);
                        for channel in 0..3 {
                            sum[channel] += rgb[channel] as usize;
                        }
                        covered += 1;
                    }
                }
            }
            if covered * 100 >= (x1 - x0) * (y1 - y0) * 55 {
                out[(cell_top + cy) * LOOK_WIDTH + cell_left + cx] = Some(sum.map(|total| (total / covered) as u8));
            }
        }
    }
    out
}

/// Median cut down to `size` colors.
fn palette(colors: &[[u8; 3]], size: usize) -> Vec<[u8; 3]> {
    let mut boxes = vec![colors.to_vec()];
    while boxes.len() < size {
        let widest = boxes
            .iter()
            .enumerate()
            .filter(|(_, colors)| colors.len() > 1)
            .map(|(index, colors)| {
                let range = |channel: usize| {
                    let values = colors.iter().map(|color| color[channel]);
                    values.clone().max().unwrap() - values.min().unwrap()
                };
                let channel = (0..3).max_by_key(|&channel| (range(channel), std::cmp::Reverse(channel))).unwrap();
                (index, channel, range(channel))
            })
            .max_by_key(|&(index, _, range)| (range, std::cmp::Reverse(index)));
        let Some((index, channel, _)) = widest else { break };
        let mut sorted = boxes[index].clone();
        sorted.sort_by_key(|color| color[channel]);
        let upper = sorted.split_off(sorted.len() / 2);
        boxes[index] = sorted;
        boxes.push(upper);
    }
    boxes
        .iter()
        .filter(|colors| !colors.is_empty())
        .map(|colors| {
            let n = colors.len();
            [0, 1, 2].map(|channel| (colors.iter().map(|color| color[channel] as usize).sum::<usize>() / n) as u8)
        })
        .collect()
}

fn rgb565([r, g, b]: [u8; 3]) -> u16 {
    ((r as u16 >> 3) << 11) | ((g as u16 >> 2) << 5) | (b as u16 >> 3)
}

/// One to four drawings (normal, eyes closed, happy, sad; missing ones reuse the normal one) as a look.
/// None when no figure can be found in one of them.
pub fn build_look(images: &[Image]) -> Option<Vec<u8>> {
    if images.is_empty() || images.len() > 4 {
        return None;
    }
    let masks: Vec<Vec<bool>> = images.iter().map(opaque_mask).collect();
    let frames: Vec<usize> = (0..4).map(|frame| if frame < images.len() { frame } else { 0 }).collect();
    let (_, top, _, bottom) = bounding_box(&images[0], &masks[0])?;
    let scale = FIGURE_HEIGHT as f64 / (bottom - top) as f64;
    let mut cells = Vec::new();
    for &index in &frames {
        let figure = bounding_box(&images[index], &masks[index])?;
        cells.push(cell(&images[index], &masks[index], figure, scale));
    }
    let opaque: Vec<[u8; 3]> = cells.iter().flatten().filter_map(|pixel| *pixel).collect();
    let colors = palette(&opaque, LOOK_COLORS - 1);
    let nearest = |color: [u8; 3]| {
        let distance = |entry: &[u8; 3]| (0..3).map(|c| (color[c] as i32 - entry[c] as i32).pow(2)).sum::<i32>();
        (0..colors.len()).min_by_key(|&index| distance(&colors[index])).unwrap() as u8 + 1
    };
    let mut out = b"LOOK".to_vec();
    out.extend_from_slice(&[LOOK_WIDTH as u8, LOOK_HEIGHT as u8, 4, 0]);
    for entry in 0..LOOK_COLORS {
        let color = if entry == 0 { 0 } else { colors.get(entry - 1).map_or(0, |&color| rgb565(color)) };
        out.extend_from_slice(&color.to_le_bytes());
    }
    for cell in &cells {
        let indices: Vec<u8> = cell.iter().map(|pixel| pixel.map_or(0, nearest)).collect();
        out.extend(indices.chunks(2).map(|pair| pair[0] | (pair[1] << 4)));
    }
    Some(out)
}

/// A look's four frames as RGBA images, for showing it; None if it isn't a look.
pub fn look_frames(look: &[u8]) -> Option<Vec<Image>> {
    if look.len() != LOOK_BYTES || &look[..4] != b"LOOK" {
        return None;
    }
    let palette: Vec<[u8; 3]> = (0..LOOK_COLORS)
        .map(|index| {
            let entry = u16_at(look, 8 + index * 2);
            [((entry >> 11) * 255 / 31) as u8, (((entry >> 5) & 0x3F) * 255 / 63) as u8, ((entry & 0x1F) * 255 / 31) as u8]
        })
        .collect();
    let frames = (0..4)
        .map(|frame| {
            let mut pixels = vec![0u8; LOOK_WIDTH * LOOK_HEIGHT * 4];
            for at in 0..LOOK_WIDTH * LOOK_HEIGHT {
                let byte = look[8 + LOOK_COLORS * 2 + frame * LOOK_FRAME_BYTES + at / 2];
                let index = if at % 2 == 0 { byte & 15 } else { byte >> 4 } as usize;
                if index != 0 {
                    let [r, g, b] = palette[index];
                    pixels[at * 4..at * 4 + 4].copy_from_slice(&[r, g, b, 255]);
                }
            }
            Image { width: LOOK_WIDTH, height: LOOK_HEIGHT, pixels }
        })
        .collect();
    Some(frames)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn shipped(path: &str) -> Option<Vec<u8>> {
        std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../characters").join(path)).ok()
    }

    #[test]
    fn a_shipped_pack_rebuilds_byte_for_byte() {
        for id in ["ada", "wanwanxiaohe"] {
            let data = shipped(&format!("{id}/pack.bin")).expect("the pack is in the repo");
            let pack = Pack::parse(&data).expect("a Character pack");
            assert_eq!(pack.id, id);
            assert!(pack.look.is_some(), "{id} wears a look");
            assert_eq!(pack.build().as_deref(), Some(&data[..]), "{id}");
        }
    }

    #[test]
    fn a_form_of_address_replaces_the_first_lines_of_each_pool() {
        let base = Pack::parse(&shipped("wanwanxiaohe/pack.bin").unwrap()).unwrap();
        let variant = Pack::parse(&shipped("wanwanxiaohe/address/ge.bin").unwrap()).unwrap();
        let addressed = base.clone().with_address(&variant);
        for (occasion, (pool, lines)) in addressed.pools.iter().zip(&variant.pools).enumerate() {
            assert_eq!(&pool[..lines.len()], &lines[..], "occasion {occasion}: the variant's lines come first");
            assert_eq!(&pool[lines.len()..], &base.pools[occasion][lines.len()..], "occasion {occasion}: the rest stay");
        }
        assert_eq!(addressed.look, base.look);
        let rebuilt = Pack::parse(&addressed.build().unwrap()).unwrap();
        assert_eq!(rebuilt, addressed);
    }

    /// A white background with an orange 20 × 60 block, two white pixels inside for eyes.
    fn drawing() -> Image {
        let mut pixels = vec![255u8; 100 * 100 * 4];
        for y in 20..80 {
            for x in 40..60 {
                let at = (y * 100 + x) * 4;
                let eye = y == 30 && (x == 45 || x == 54);
                pixels[at..at + 3].copy_from_slice(if eye { &[255, 255, 255] } else { &[250, 120, 20] });
            }
        }
        Image { width: 100, height: 100, pixels }
    }

    #[test]
    fn a_drawing_becomes_a_look_bottom_aligned_and_cut_out() {
        let look = build_look(&[drawing()]).expect("a figure");
        assert_eq!(look.len(), LOOK_BYTES);
        let frames = look_frames(&look).unwrap();
        assert_eq!(frames.len(), 4);
        let alpha = |x: usize, y: usize| frames[0].pixels[(y * LOOK_WIDTH + x) * 4 + 3];
        assert_eq!(alpha(0, 0), 0, "the background is gone");
        assert_eq!(alpha(24, 40), 255, "the figure is there");
        assert_eq!(alpha(24, 3), 0, "60 tall in a 64-tall cell: the top rows are empty");
        assert_eq!(alpha(24, 63), 255, "bottom-aligned");
        assert_eq!(frames[3], frames[0], "a single drawing stands in for all four");
    }

    #[test]
    fn a_custom_character_borrows_voice_and_lines() {
        let base = Pack::parse(&shipped("wanwanxiaohe/pack.bin").unwrap()).unwrap();
        let look = build_look(&[drawing()]).unwrap();
        let custom = base.clone().with_look(look.clone(), "custom");
        let parsed = Pack::parse(&custom.build().unwrap()).unwrap();
        assert_eq!(parsed.id, "custom");
        assert_eq!(parsed.look.as_deref(), Some(&look[..]));
        assert_eq!(parsed.pools, base.pools);
    }

    #[test]
    fn crc32_is_zlibs() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }
}
