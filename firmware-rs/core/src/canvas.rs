//! 320×240 的 RGB565 帧缓冲与最基本的画法：填矩形、粗线、5×7 点阵字。
//!
//! 缓冲按屏幕要的字节序排：每个像素高字节在前。C 固件在内存里存小端 u16，
//! 让 LCD 驱动发送时交换字节；这里直接存成线上的样子，DMA 拿去就发。

pub const WIDTH: i32 = 320;
pub const HEIGHT: i32 = 240;
pub const FRAME_BYTES: usize = (WIDTH * HEIGHT * 2) as usize;

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

/// 一个字节的一行点阵。没有字形的字节（包括 UTF-8 多字节字符的每个字节）
/// 画成一个小问号似的占位符，和 C 固件一样按字节画。
fn glyph_row(character: u8, row: usize) -> u8 {
    let upper = character.to_ascii_uppercase();
    match upper {
        b'A'..=b'Z' => LETTER_GLYPHS[(upper - b'A') as usize][row],
        b'0'..=b'9' => DIGIT_GLYPHS[(upper - b'0') as usize][row],
        b'-' => if row == 3 { 0x1f } else { 0 },
        b'.' => if row == 6 { 0x04 } else { 0 },
        b':' => if row == 2 || row == 5 { 0x04 } else { 0 },
        b'/' => 1 << if row < 5 { 4 - row } else { 0 },
        b'!' => if row < 5 || row == 6 { 0x04 } else { 0 },
        b'?' => [0x0e, 0x11, 0x01, 0x02, 0x04, 0x00, 0x04][row],
        b'>' => [0x10, 0x08, 0x04, 0x02, 0x04, 0x08, 0x10][row],
        b' ' => 0,
        b'_' => if row == 6 { 0x1f } else { 0 },
        _ => match row {
            0 | 3 => 0x0e,
            1 | 2 => 0x11,
            _ => 0x04,
        },
    }
}

/// RGB565 各分量减半。
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

    /// 三像素粗的 Bresenham 线。
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

    /// 从 (x, y) 起按字节画，最多 `max_characters` 个；遇到 NUL 停。
    pub fn draw_text(&mut self, x: i32, y: i32, text: &[u8], scale: i32, color: u16, max_characters: usize) {
        for (index, &character) in text.iter().take(max_characters).enumerate() {
            if character == 0 {
                break;
            }
            for row in 0..7 {
                let bits = glyph_row(character, row);
                for column in 0..5 {
                    if bits & (1 << (4 - column)) != 0 {
                        self.fill_rect(
                            x + index as i32 * 6 * scale + column * scale,
                            y + row as i32 * scale,
                            scale,
                            scale,
                            color,
                        );
                    }
                }
            }
        }
    }

    pub fn draw_text_centered(&mut self, y: i32, text: &[u8], scale: i32, color: u16) {
        let max_characters = (WIDTH / (6 * scale)) as usize;
        let length = text.iter().take(max_characters).take_while(|&&byte| byte != 0).count() as i32;
        let width = if length == 0 { 0 } else { (length * 6 - 1) * scale };
        self.draw_text((WIDTH - width) / 2, y, text, scale, color, max_characters);
    }

    /// 困倦期整屏转暗：每个像素各分量减半。
    pub fn dim(&mut self) {
        for pair in self.bytes[..FRAME_BYTES].as_chunks_mut::<2>().0 {
            let dimmed = half_bright(u16::from_be_bytes([pair[0], pair[1]])).to_be_bytes();
            pair.copy_from_slice(&dimmed);
        }
    }
}
