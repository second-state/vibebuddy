//! The touchscreen some boxes have (docs/hardware.md, "Touch, and what else is on I2C0"; ADR-0017): a CHSC5432 at
//! 0x2E on I2C0. Others have nothing there, so the device layer probes it at boot and the box does without.
//!
//! The chip uses 32-bit register addresses, written big-endian before a read. This module turns its report into a
//! point on the screen and a stream of samples into taps, the one touch gesture; reading I2C is the device layer's
//! job.

/// 7-bit I2C address.
pub const ADDRESS: u8 = 0x2E;
/// Where the touch report is read from.
pub const REPORT_REGISTER: u32 = 0x2000_002C;
/// Where the chip ID is read from; answering here is how the probe knows the chip is there.
pub const ID_REGISTER: u32 = 0x2000_0080;
/// The vendor reads the report in one go of this many bytes.
pub const REPORT_BYTES: usize = 28;
/// How often the device layer reads the report. There's no interrupt line we know of, and a tap is
/// rarely shorter than 80 ms, so every 20 ms sees it at least a few times.
pub const POLL_MS: u32 = 20;

/// Down and up again within this long...
const TAP_MS: u32 = 400;
/// ...without straying further than this from where it went down, in either direction.
const TAP_SLOP: i32 = 12;

const WIDTH: i32 = 320;
const HEIGHT: i32 = 240;

/// A point on the screen, in the canvas's coordinates (320×240, origin top left).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

/// The first touch point in a report, or None when no finger is down.
///
/// Byte 1 holds the number of points. Each point takes five bytes from byte 2: the low bytes of the two
/// coordinates in bytes 2 and 3, and both high nibbles in byte 5. The chip reports in the panel's portrait frame
/// (240 wide); ALIENTEK's driver for the same panel in landscape takes byte 3 as x and counts y from the bottom
/// (example-idf/touch.md in openedv-wiki-boards-dnesp32s3b3), which is what is done here.
pub fn parse_report(report: &[u8; REPORT_BYTES]) -> Option<Point> {
    let points = report[1] & 0x0F;
    if points == 0 {
        return None;
    }
    let short = (i32::from(report[5] & 0x0F) << 8) | i32::from(report[2]);
    let long = (i32::from(report[5] >> 4) << 8) | i32::from(report[3]);
    let point = Point { x: long, y: HEIGHT - 1 - short };
    // Off the screen is the chip's way of saying it has nothing good.
    if !(0..WIDTH).contains(&point.x) || !(0..HEIGHT).contains(&point.y) {
        return None;
    }
    Some(point)
}

/// Turns samples into taps: a finger down and up again quickly, in about the same place.
#[derive(Default)]
pub struct Taps {
    /// Where and when the finger went down, and whether it has strayed or stayed too long since.
    down: Option<(Point, u32, bool)>,
}

impl Taps {
    pub fn new() -> Self {
        Self::default()
    }

    /// One sample: where the finger is, or None when it's up. Returns the tap when one ends here, at the place
    /// the finger went down (the place the user aimed at, before any slide on the way up).
    pub fn update(&mut self, sample: Option<Point>, now_ms: u32) -> Option<Point> {
        match (sample, &mut self.down) {
            (Some(point), None) => {
                self.down = Some((point, now_ms, false));
                None
            }
            (Some(point), Some((start, since, spoiled))) => {
                let strayed = (point.x - start.x).abs() > TAP_SLOP || (point.y - start.y).abs() > TAP_SLOP;
                if strayed || now_ms.wrapping_sub(*since) > TAP_MS {
                    *spoiled = true;
                }
                None
            }
            (None, Some((start, since, spoiled))) => {
                let tap = (!*spoiled && now_ms.wrapping_sub(*since) <= TAP_MS).then_some(*start);
                self.down = None;
                tap
            }
            (None, None) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A report as the chip sends it, with one point at the given raw coordinates.
    fn report(short: u16, long: u16) -> [u8; REPORT_BYTES] {
        let mut bytes = [0u8; REPORT_BYTES];
        bytes[1] = 1;
        bytes[2] = short as u8;
        bytes[3] = long as u8;
        bytes[5] = ((long >> 8) as u8) << 4 | (short >> 8) as u8;
        bytes
    }

    #[test]
    fn a_report_with_no_points_is_no_touch() {
        assert_eq!(parse_report(&[0; REPORT_BYTES]), None);
        // A finger lifting: event 0x40 (up) and no points, as the probe saw on box 30:ED:A0:A4:0D:08.
        let mut up = report(100, 200);
        up[1] = 0;
        up[6] = 0x40;
        assert_eq!(parse_report(&up), None);
    }

    #[test]
    fn the_high_nibbles_carry_coordinates_past_255() {
        assert_eq!(parse_report(&report(0, 300)), Some(Point { x: 300, y: 239 }));
        assert_eq!(parse_report(&report(239, 0)), Some(Point { x: 0, y: 0 }));
        assert_eq!(parse_report(&report(120, 260)), Some(Point { x: 260, y: 119 }));
    }

    #[test]
    fn a_point_off_the_screen_is_ignored() {
        assert_eq!(parse_report(&report(0, 320)), None);
        assert_eq!(parse_report(&report(240, 10)), None);
        assert_eq!(parse_report(&[0xFF; REPORT_BYTES]), None, "what a read without a register address gives");
    }

    const AT: Point = Point { x: 100, y: 80 };

    #[test]
    fn down_and_up_quickly_is_a_tap_where_it_went_down() {
        let mut taps = Taps::new();
        assert_eq!(taps.update(Some(AT), 0), None);
        assert_eq!(taps.update(Some(Point { x: 105, y: 84 }), 60), None);
        assert_eq!(taps.update(None, 120), Some(AT));
        assert_eq!(taps.update(None, 140), None, "one tap, once");
    }

    #[test]
    fn a_slide_or_a_hold_is_not_a_tap() {
        let mut taps = Taps::new();
        taps.update(Some(AT), 0);
        taps.update(Some(Point { x: 130, y: 80 }), 40);
        taps.update(Some(AT), 80);
        assert_eq!(taps.update(None, 100), None, "it strayed, even though it came back");

        taps.update(Some(AT), 1000);
        taps.update(Some(AT), 1500);
        assert_eq!(taps.update(None, 1520), None, "held too long");

        taps.update(Some(AT), 2000);
        assert_eq!(taps.update(None, 2500), None, "held too long between two samples");
    }
}
