//! Text beyond ASCII: Chinese from a 12 px pixel font, next to the 5×7 ASCII font in canvas.rs.
//!
//! The glyphs are the 3,500 characters of level one of the Table of General Standard Chinese Characters
//! (通用规范汉字表 一级) plus CJK punctuation and the full-width forms, from Fusion Pixel Font 12 px
//! (SIL OFL 1.1, fonts/OFL.txt). `tools/make-cjk-font.py` builds the table: a sorted index of codepoints
//! and one packed 12×12 bitmap per character, 18 bytes each. It is included as read-only data, so it
//! stays in flash and costs no RAM.
//!
//! ASCII keeps the 5×7 font, so English looks as it always has. A Chinese character is one 12 px cell at
//! scales 1 and 2 (two cells at 4), vertically centred on the ASCII line; at scale 1 that puts the font's
//! own baseline exactly on the ASCII baseline. A character the table lacks draws as a dotted box.

use alloc::vec::Vec;

static TABLE: &[u8] = include_bytes!("../fonts/cjk-12px.bin");

/// A wide glyph's cell: 12 pixels square, ink in rows 1 to 11 and columns 0 to 10.
pub const CELL: i32 = 12;
const GLYPH_BYTES: usize = 18;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    /// A byte for the 5×7 font, which draws its own placeholder for bytes it has no glyph for.
    Ascii(u8),
    /// An index into the table.
    Wide(usize),
    /// Not in the table, or not UTF-8: a box the width of a wide glyph.
    Missing,
}

fn count() -> usize {
    u16::from_le_bytes([TABLE[0], TABLE[1]]) as usize
}

fn codepoint(index: usize) -> u32 {
    u16::from_le_bytes([TABLE[2 + index * 2], TABLE[3 + index * 2]]) as u32
}

/// The table index of a character, by binary search.
pub fn lookup(character: char) -> Option<usize> {
    let wanted = character as u32;
    let (mut low, mut high) = (0, count());
    while low < high {
        let middle = (low + high) / 2;
        match codepoint(middle).cmp(&wanted) {
            core::cmp::Ordering::Less => low = middle + 1,
            core::cmp::Ordering::Greater => high = middle,
            core::cmp::Ordering::Equal => return Some(middle),
        }
    }
    None
}

/// One row of a wide glyph, 12 bits, the leftmost pixel in bit 11.
pub fn row(index: usize, row: usize) -> u16 {
    let bitmap = &TABLE[2 + count() * 2 + index * GLYPH_BYTES..][..GLYPH_BYTES];
    let at = row * 3 / 2;
    let (first, second) = (bitmap[at] as u16, bitmap[at + 1] as u16);
    if row.is_multiple_of(2) { first << 4 | second >> 4 } else { (first & 0x0f) << 8 | second }
}

/// A wide glyph's pixel size at a text scale: one pixel up to scale 3, so Chinese stays 12 px next to both
/// small and large ASCII.
pub fn wide_scale(scale: i32) -> i32 {
    (scale / 2).max(1)
}

/// Where a wide glyph's cell starts for ASCII drawn at `y`: centred on the 7-row ASCII glyph.
pub fn wide_top(y: i32, scale: i32) -> i32 {
    y + (7 * scale - CELL * wide_scale(scale)).div_euclid(2)
}

pub fn advance(glyph: Glyph, scale: i32) -> i32 {
    match glyph {
        Glyph::Ascii(_) => 6 * scale,
        _ => CELL * wide_scale(scale),
    }
}

/// The blank column at the right of every glyph, which a measured width leaves off the last one.
fn gap(glyph: Glyph, scale: i32) -> i32 {
    match glyph {
        Glyph::Ascii(_) => scale,
        _ => wide_scale(scale),
    }
}

/// The glyphs of UTF-8 text, each with its length in bytes; stops at NUL like the 5×7 drawing does.
pub fn glyphs(text: &[u8]) -> Glyphs<'_> {
    Glyphs { text, at: 0, previous_alphanumeric: false }
}

pub struct Glyphs<'a> {
    text: &'a [u8],
    at: usize,
    previous_alphanumeric: bool,
}

impl Iterator for Glyphs<'_> {
    type Item = (Glyph, usize);

    fn next(&mut self) -> Option<Self::Item> {
        let byte = *self.text.get(self.at)?;
        if byte == 0 {
            return None;
        }
        let (glyph, length) = if byte.is_ascii() {
            (Glyph::Ascii(byte), 1)
        } else {
            let length = match byte {
                0xc2..=0xdf => 2,
                0xe0..=0xef => 3,
                0xf0..=0xf4 => 4,
                _ => 0,
            };
            let character = self.text.get(self.at..self.at + length).and_then(|bytes| core::str::from_utf8(bytes).ok()?.chars().next());
            match character {
                Some(character) => {
                    let next_alphanumeric = self.text.get(self.at + length).is_some_and(u8::is_ascii_alphanumeric);
                    (self.wide(character, next_alphanumeric), length)
                }
                None => {
                    // One box for a broken sequence, not one per byte.
                    let mut length = 1;
                    while self.text.get(self.at + length).is_some_and(|&byte| byte & 0xc0 == 0x80) {
                        length += 1;
                    }
                    (Glyph::Missing, length)
                }
            }
        };
        self.at += length;
        self.previous_alphanumeric = byte.is_ascii_alphanumeric();
        Some((glyph, length))
    }
}

impl Glyphs<'_> {
    fn wide(&self, character: char, next_alphanumeric: bool) -> Glyph {
        // Curly single quotes against English letters are apostrophes ("it’s"): the ASCII one fits there,
        // a full-width quote would leave a gap.
        if matches!(character, '\u{2018}' | '\u{2019}') && (self.previous_alphanumeric || next_alphanumeric) {
            return Glyph::Ascii(b'\'');
        }
        lookup(character).map_or(Glyph::Missing, Glyph::Wide)
    }
}

/// Pixel width of text as drawn at `scale`: up to the last glyph's ink, without its trailing gap.
pub fn width(text: &[u8], scale: i32) -> i32 {
    let mut width = 0;
    let mut last_gap = 0;
    for (glyph, _) in glyphs(text) {
        width += advance(glyph, scale);
        last_gap = gap(glyph, scale);
    }
    width - last_gap
}

/// How many bytes of `text` fit in `max_width` pixels, whole characters only.
pub fn fit(text: &[u8], scale: i32, max_width: i32) -> usize {
    let mut used = 0;
    let mut bytes = 0;
    for (glyph, length) in glyphs(text) {
        if used + advance(glyph, scale) - gap(glyph, scale) > max_width {
            break;
        }
        used += advance(glyph, scale);
        bytes += length;
    }
    bytes
}

/// Whether any of the text draws in wide glyphs, which need taller rows than ASCII alone.
pub fn has_wide(text: &[u8]) -> bool {
    glyphs(text).any(|(glyph, _)| !matches!(glyph, Glyph::Ascii(_)))
}

/// Punctuation that must not start a row: rather than move it down alone, it hangs past the row's end.
fn closing(text: &[u8]) -> bool {
    core::str::from_utf8(text)
        .is_ok_and(|text| matches!(text, "，" | "。" | "、" | "；" | "：" | "！" | "？" | "）" | "》" | "〉" | "」" | "』" | "】" | "〕" | "”" | "’" | "…"))
}

/// Wraps text to rows of at most `max_width` pixels at `scale`. Breaks at spaces, and between any two
/// Chinese characters or a Chinese character and a word; an ASCII word is only cut when it is longer than
/// a row. Runs of whitespace become one space. Closing Chinese punctuation that doesn't fit hangs one
/// glyph past `max_width` instead of starting a row.
pub fn wrap(text: &str, max_width: i32, scale: i32) -> Vec<Vec<u8>> {
    let bytes = text.as_bytes();
    let mut rows: Vec<Vec<u8>> = Vec::new();
    let mut row: Vec<u8> = Vec::new();
    let mut at = 0;
    let mut space = false;
    let place = |unit: &[u8], space: bool, rows: &mut Vec<Vec<u8>>, row: &mut Vec<u8>| {
        let mut unit = unit;
        if !row.is_empty() {
            let mut candidate = row.clone();
            if space {
                candidate.push(b' ');
            }
            candidate.extend_from_slice(unit);
            if width(&candidate, scale) <= max_width || (!space && closing(unit)) {
                *row = candidate;
                return;
            }
            rows.push(core::mem::take(row));
        }
        while width(unit, scale) > max_width {
            let first = glyphs(unit).next().map_or(unit.len(), |(_, length)| length);
            let cut = fit(unit, scale, max_width).max(first);
            rows.push(unit[..cut].to_vec());
            unit = &unit[cut..];
        }
        *row = unit.to_vec();
    };
    let mut iterator = glyphs(bytes).peekable();
    while let Some((glyph, length)) = iterator.next() {
        let start = at;
        at += length;
        match glyph {
            Glyph::Ascii(byte) if byte.is_ascii_whitespace() => {
                space = true;
                continue;
            }
            Glyph::Ascii(_) => {
                // A word: ASCII up to the next space or wide glyph.
                while let Some(&(Glyph::Ascii(byte), length)) = iterator.peek() {
                    if byte.is_ascii_whitespace() {
                        break;
                    }
                    at += length;
                    iterator.next();
                }
            }
            _ => {}
        }
        place(&bytes[start..at], space, &mut rows, &mut row);
        space = false;
    }
    if !row.is_empty() {
        rows.push(row);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::String;
    use alloc::vec;

    fn rows(text: &str, max_width: i32, scale: i32) -> Vec<String> {
        wrap(text, max_width, scale).into_iter().map(|row| String::from_utf8(row).unwrap()).collect()
    }

    #[test]
    fn the_table_holds_level_one_and_punctuation_in_order() {
        assert!(count() > 3500);
        assert_eq!(TABLE.len(), 2 + count() * (2 + GLYPH_BYTES));
        assert!((1..count()).all(|index| codepoint(index - 1) < codepoint(index)));
        for character in "你好世界的一了是我龟鼠，。、！？：；“”‘’（）《》…—".chars() {
            assert!(lookup(character).is_some(), "{character}");
        }
        // Level two and beyond, and other scripts, are not in it.
        assert_eq!(lookup('龘'), None);
        assert_eq!(lookup('é'), None);
        assert_eq!(lookup('A'), None);
    }

    #[test]
    fn a_glyph_keeps_its_spacing_row_and_column_blank() {
        let index = lookup('国').unwrap();
        assert_eq!(row(index, 0), 0, "the top row is the line spacing");
        assert!((0..12).all(|line| row(index, line) & 1 == 0), "the right column is the character spacing");
        assert!((1..12).filter(|&line| row(index, line) != 0).count() >= 9);
        // 一 is a single stroke across the middle.
        let one = lookup('一').unwrap();
        assert_eq!((0..12).filter(|&line| row(one, line) != 0).count(), 1);
    }

    #[test]
    fn utf8_turns_into_glyphs_with_their_byte_lengths() {
        let found: Vec<(Glyph, usize)> = glyphs("a你\u{1F600}".as_bytes()).collect();
        assert_eq!(found, [(Glyph::Ascii(b'a'), 1), (Glyph::Wide(lookup('你').unwrap()), 3), (Glyph::Missing, 4)]);
        // A broken sequence is one box; NUL ends the text.
        assert_eq!(glyphs(&[0xe4, 0xbd, b'x', 0, b'y']).collect::<Vec<_>>(), [(Glyph::Missing, 2), (Glyph::Ascii(b'x'), 1)]);
        assert_eq!(glyphs(&[0xff]).collect::<Vec<_>>(), [(Glyph::Missing, 1)]);
    }

    #[test]
    fn curly_quotes_by_english_letters_are_apostrophes() {
        assert_eq!(glyphs("it’s".as_bytes()).nth(2), Some((Glyph::Ascii(b'\''), 3)));
        assert_eq!(glyphs("‘ok".as_bytes()).next(), Some((Glyph::Ascii(b'\''), 3)));
        assert_eq!(glyphs("说‘好’".as_bytes()).nth(1), Some((Glyph::Wide(lookup('‘').unwrap()), 3)));
    }

    #[test]
    fn mixed_text_measures_each_font_by_its_own_advance() {
        assert_eq!(width(b"", 1), 0);
        assert_eq!(width(b"ABC", 1), 17, "the 5x7 font: 6 px a character, less the last gap");
        assert_eq!(width(b"ABC", 2), 34);
        assert_eq!(width("你好".as_bytes(), 1), 23);
        assert_eq!(width("你好".as_bytes(), 2), 23, "12 px next to large ASCII too");
        assert_eq!(width("OK了".as_bytes(), 1), 12 + 11);
        assert_eq!(width("\u{1F600}".as_bytes(), 1), 11, "a missing character is as wide as a Chinese one");
        assert_eq!(fit("你好吗".as_bytes(), 1, 30), 6);
        assert_eq!(fit("你好吗".as_bytes(), 1, 35), 9);
        assert_eq!(fit(b"ABCDE", 1, 17), 3);
        assert!(has_wide("ok，".as_bytes()));
        assert!(!has_wide("it’s ok".as_bytes()));
    }

    #[test]
    fn ascii_wraps_at_spaces_and_cuts_only_long_words() {
        let ascii = |text: &str, columns: i32| rows(text, columns * 6 - 1, 1);
        assert_eq!(ascii("a bb ccc", 4), ["a bb", "ccc"]);
        assert_eq!(ascii("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        assert_eq!(ascii("x abcdefghij y", 4), ["x", "abcd", "efgh", "ij y"]);
        assert_eq!(ascii("  spaced\nout  ", 20), ["spaced out"]);
        assert_eq!(ascii("it’s", 10), ["it’s"]);
        assert!(wrap("", 100, 1).is_empty());
    }

    #[test]
    fn chinese_breaks_between_any_two_characters_but_keeps_words_whole() {
        // Four characters to a row: 4 * 12 - 1.
        assert_eq!(rows("一二三四五六七八九", 47, 1), ["一二三四", "五六七八", "九"]);
        assert_eq!(rows("打开Codex看看", 47, 1), ["打开", "Codex看", "看"]);
        assert_eq!(rows("你好 world", 47, 1), ["你好", "world"]);
        // Closing punctuation hangs past the end rather than starting a row.
        assert_eq!(rows("一二三四。五", 47, 1), ["一二三四。", "五"]);
        assert_eq!(rows("一二三四 。", 47, 1), ["一二三四", "。"], "unless a space stands before it");
        // A missing character still takes a cell and wraps like one.
        assert_eq!(rows("一二三\u{1F600}四", 47, 1), ["一二三\u{1F600}", "四"]);
    }

    #[test]
    fn a_row_narrower_than_one_glyph_still_moves_on() {
        assert_eq!(rows("你好", 5, 1), ["你", "好"]);
        assert_eq!(wrap("ab", 3, 1), vec![b"a".to_vec(), b"b".to_vec()]);
    }
}
