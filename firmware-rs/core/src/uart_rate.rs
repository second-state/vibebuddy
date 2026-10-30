//! The UART bridge's baud rate (docs/protocol.md, "Link speed on the UART bridge").
//!
//! The box starts at 115200, which can't carry a voice command's audio. The computer on the bridge asks for a faster
//! rate with `device.baud`; the box says `BAUD SWITCH <rate>` at the old rate, moves, and keeps the new rate only when
//! a whole line arrives at it within [`TRIAL_MS`]. Whenever the two ends may no longer agree, the box goes back to
//! 115200, the rate both ends start at: the trial isn't confirmed, the UART reports bytes it couldn't frame (the
//! computer talks at another rate), or the UART stays silent while the computer would send heartbeats.
//!
//! This is only the decision; the device layer moves the UART and writes the lines.

/// The rate every box and computer starts at, and the one old firmware knows.
pub const SLOW: u32 = 115_200;
/// The rate a voice command's audio needs: about 16 KB/s of `VOICE DATA` against 92 KB/s at this rate. The CH343
/// bridge and the ESP32-S3's UART both run it.
pub const FAST: u32 = 921_600;
/// A switch is kept only if a whole line arrives at the new rate this soon.
pub const TRIAL_MS: u32 = 2_000;
/// At the fast rate, a UART this long silent means the computer left or reopened the port at 115200 without
/// sending anything yet; the computer sends a heartbeat every few seconds.
pub const SILENCE_MS: u32 = 30_000;

/// Finds bytes the computer never sends over the UART, whose lines are UTF-8 text: what bytes sent at 115200 look
/// like to a UART at 921600. They still frame cleanly, mostly as 0x80, so the UART reports no error. A 0x80 is text
/// only inside a multi-byte character, so the check follows the UTF-8 sequence across bytes.
#[derive(Debug, Default)]
pub struct TextCheck {
    /// Continuation bytes (0x80..=0xBF) the last lead byte still calls for.
    pending: u8,
}

impl TextCheck {
    pub const fn new() -> Self {
        Self { pending: 0 }
    }

    /// The next byte from the UART: true when no UTF-8 text holds it here.
    pub fn garbled(&mut self, byte: u8) -> bool {
        let pending = core::mem::take(&mut self.pending);
        match byte {
            0x80..=0xbf => {
                self.pending = pending.saturating_sub(1);
                return pending == 0;
            }
            0x00 | 0xc0 | 0xc1 | 0xf5..=0xff => return true,
            0xc2..=0xdf => self.pending = 1,
            0xe0..=0xef => self.pending = 2,
            0xf0..=0xf4 => self.pending = 3,
            _ => {}
        }
        pending != 0
    }
}

#[derive(Debug)]
pub struct UartRate {
    rate: u32,
    /// When the switch to `rate` started, until a whole line confirms it.
    trial_since: Option<u32>,
    /// When the UART last brought a whole line.
    heard: u32,
}

impl Default for UartRate {
    fn default() -> Self {
        Self::new()
    }
}

impl UartRate {
    pub const fn new() -> Self {
        Self { rate: SLOW, trial_since: None, heard: 0 }
    }

    /// The rate the UART runs at.
    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// The UART runs fast and the computer confirmed it: it can carry an audio stream.
    pub fn carries_stream(&self) -> bool {
        self.rate != SLOW && self.trial_since.is_none()
    }

    /// `device.baud`: the rate to move to after saying `BAUD SWITCH <rate>` at the current one, or None for a rate
    /// this box doesn't run.
    pub fn request(&mut self, rate: u32, now: u32) -> Option<u32> {
        if rate != SLOW && rate != FAST {
            return None;
        }
        self.rate = rate;
        self.trial_since = Some(now);
        self.heard = now;
        Some(rate)
    }

    /// A whole line came over the UART. Some(rate) when it confirmed a switch: say `BAUD <rate>`.
    pub fn heard_line(&mut self, now: u32) -> Option<u32> {
        self.heard = now;
        self.trial_since.take().map(|_| self.rate)
    }

    /// The UART received `count` bytes it couldn't frame, or ones [`TextCheck`] finds garbled. Some(115200) when the box should go
    /// back to it now: at the fast rate even one means the computer may be talking at 115200, as after it reopened
    /// the port. Its first line at 115200 is lost either way, and the computer asks again.
    pub fn receive_errors(&mut self, count: u32) -> Option<u32> {
        (count > 0 && self.rate != SLOW).then(|| self.fall_back())
    }

    /// Every pass of the main loop. Some(115200) when the box should go back to it now: the switch wasn't
    /// confirmed in time, or the UART has been silent too long at the fast rate.
    pub fn poll(&mut self, now: u32) -> Option<u32> {
        if self.rate == SLOW {
            return None;
        }
        let expired = match self.trial_since {
            Some(since) => now.wrapping_sub(since) >= TRIAL_MS,
            None => now.wrapping_sub(self.heard) >= SILENCE_MS,
        };
        expired.then(|| self.fall_back())
    }

    /// Back to 115200, as before a restart: true when the UART wasn't there already, so the computer must be told.
    pub fn reset(&mut self) -> bool {
        let moved = self.rate != SLOW;
        self.fall_back();
        moved
    }

    fn fall_back(&mut self) -> u32 {
        self.rate = SLOW;
        self.trial_since = None;
        SLOW
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_switch_is_kept_once_a_line_arrives_at_the_new_rate() {
        let mut uart = UartRate::new();
        assert_eq!(uart.request(FAST, 100), Some(FAST));
        assert!(!uart.carries_stream(), "not before the computer confirms it");
        assert_eq!(uart.poll(100 + TRIAL_MS - 1), None);
        assert_eq!(uart.heard_line(100 + TRIAL_MS - 1), Some(FAST), "say BAUD 921600");
        assert!(uart.carries_stream());
        assert_eq!(uart.heard_line(5_000), None, "said once");
        assert_eq!(uart.rate(), FAST);
    }

    #[test]
    fn an_unconfirmed_switch_goes_back_to_115200() {
        let mut uart = UartRate::new();
        uart.request(FAST, 100);
        assert_eq!(uart.poll(100 + TRIAL_MS), Some(SLOW));
        assert_eq!(uart.rate(), SLOW);
        assert!(!uart.carries_stream());
        assert_eq!(uart.poll(100_000), None, "nothing more to do at 115200");
    }

    #[test]
    fn a_rate_the_box_does_not_run_is_refused() {
        let mut uart = UartRate::new();
        assert_eq!(uart.request(2_000_000, 0), None);
        assert_eq!(uart.request(0, 0), None);
        assert_eq!(uart.rate(), SLOW);
    }

    #[test]
    fn a_receive_error_at_the_fast_rate_means_the_computer_may_talk_at_115200() {
        let mut uart = UartRate::new();
        uart.request(FAST, 0);
        uart.heard_line(10);
        assert_eq!(uart.receive_errors(0), None);
        assert_eq!(uart.receive_errors(1), Some(SLOW));
        assert_eq!(uart.rate(), SLOW);
        assert_eq!(uart.receive_errors(100), None, "at 115200 errors change nothing");
    }

    #[test]
    fn text_is_not_garbled() {
        let mut check = TextCheck::new();
        for &byte in "{\"event\":\"device.hello\",\"title\":\"语音 😀 é\"}\n".as_bytes() {
            assert!(!check.garbled(byte), "{byte:#04x}");
        }
    }

    #[test]
    fn bytes_sent_at_115200_read_at_921600_are_garbled() {
        // What a box at 921600 read when the daemon restarted and said hello at 115200.
        let mut check = TextCheck::new();
        check.garbled(b'\n');
        assert!(check.garbled(0x80), "a continuation byte no lead byte called for");
        let mut check = TextCheck::new();
        assert!(check.garbled(0x00));
        assert!(check.garbled(0xc0));
        assert!(check.garbled(0xf5));
        assert!(check.garbled(0xff));
    }

    #[test]
    fn a_character_cut_short_is_garbled() {
        let mut check = TextCheck::new();
        assert!(!check.garbled(0xe8));
        assert!(!check.garbled(0xaf));
        assert!(check.garbled(b'a'), "two continuation bytes called for, one came");
        assert!(!check.garbled(b'b'), "the check starts again after it");
    }

    #[test]
    fn errors_during_the_trial_end_it_at_once() {
        let mut uart = UartRate::new();
        uart.request(FAST, 0);
        assert_eq!(uart.receive_errors(2), Some(SLOW));
        assert_eq!(uart.heard_line(50), None, "nothing left to confirm");
    }

    #[test]
    fn a_silent_uart_at_the_fast_rate_goes_back_to_115200() {
        let mut uart = UartRate::new();
        uart.request(FAST, 0);
        uart.heard_line(10);
        assert_eq!(uart.poll(10 + SILENCE_MS - 1), None);
        assert_eq!(uart.poll(10 + SILENCE_MS), Some(SLOW));
    }

    #[test]
    fn the_clock_wrapping_does_not_end_the_trial_early() {
        let mut uart = UartRate::new();
        uart.request(FAST, u32::MAX - 10);
        assert_eq!(uart.poll(5), None);
        assert_eq!(uart.heard_line(5), Some(FAST));
    }

    #[test]
    fn a_restart_says_so_only_when_the_rate_moved() {
        let mut uart = UartRate::new();
        assert!(!uart.reset());
        uart.request(FAST, 0);
        uart.heard_line(1);
        assert!(uart.reset());
        assert_eq!(uart.rate(), SLOW);
    }
}
