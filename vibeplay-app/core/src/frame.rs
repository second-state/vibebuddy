//! The VibePlay's 640 × 480 frame (QingOS ADR 0036): the box's 320 × 240 frame at twice the size. Both are RGB565
//! with the high byte first, so each pixel is copied as is, four times.

use vibebuddy_firmware_core::canvas::{FRAME_BYTES, HEIGHT, WIDTH};

pub const BIG_WIDTH: usize = WIDTH as usize * 2;
pub const BIG_HEIGHT: usize = HEIGHT as usize * 2;
pub const BIG_BYTES: usize = FRAME_BYTES * 4;

/// `small` (320 × 240) into `big` (640 × 480); None for `small` draws black, as the box's screen is with its
/// backlight off.
pub fn double(small: Option<&[u8]>, big: &mut [u8]) {
    let Some(small) = small else {
        big.fill(0);
        return;
    };
    let row_bytes = WIDTH as usize * 2;
    for y in 0..HEIGHT as usize {
        let source = &small[y * row_bytes..][..row_bytes];
        let top = &mut big[y * 2 * row_bytes * 2..][..row_bytes * 2];
        for (pixel, out) in source.as_chunks::<2>().0.iter().zip(top.as_chunks_mut::<4>().0) {
            *out = [pixel[0], pixel[1], pixel[0], pixel[1]];
        }
        let (upper, lower) = big[y * 2 * row_bytes * 2..][..row_bytes * 4].split_at_mut(row_bytes * 2);
        lower.copy_from_slice(upper);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_pixel_becomes_four() {
        let mut small = vec![0u8; FRAME_BYTES];
        small[0..2].copy_from_slice(&[0x12, 0x34]);
        let last = FRAME_BYTES - 2;
        small[last..].copy_from_slice(&[0xab, 0xcd]);
        let mut big = vec![9u8; BIG_BYTES];
        double(Some(&small), &mut big);
        let at = |x: usize, y: usize| [big[(y * BIG_WIDTH + x) * 2], big[(y * BIG_WIDTH + x) * 2 + 1]];
        assert_eq!([at(0, 0), at(1, 0), at(0, 1), at(1, 1)], [[0x12, 0x34]; 4]);
        assert_eq!(at(2, 0), [0, 0]);
        assert_eq!([at(638, 478), at(639, 479)], [[0xab, 0xcd]; 2]);
        double(None, &mut big);
        assert!(big.iter().all(|&byte| byte == 0));
    }
}
