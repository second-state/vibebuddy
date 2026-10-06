//! Drawing a line from an occasion's pool: at random, but never one of the last few said for that
//! occasion, so a pool of eight doesn't sound like a pool of two.

use crate::audio::OCCASIONS;

/// How many recent lines of an occasion are kept out of the draw.
const MEMORY: usize = 3;

pub struct LinePicker {
    state: u32,
    /// Per occasion, the most recent picks, newest first; `u8::MAX` is empty.
    recent: [[u8; MEMORY]; OCCASIONS],
}

impl LinePicker {
    pub fn new(seed: u32) -> Self {
        Self { state: seed | 1, recent: [[u8::MAX; MEMORY]; OCCASIONS] }
    }

    fn next_random(&mut self) -> u32 {
        // xorshift32, as in the leisure director.
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state = x;
        x
    }

    /// Picks an index into a pool of `size` lines for `occasion`. A pool of n lines keeps the last
    /// min(3, n − 1) out, so even a pool of two alternates.
    pub fn pick(&mut self, occasion: usize, size: usize) -> usize {
        if size <= 1 {
            return 0;
        }
        let size = size.min(u8::MAX as usize);
        let kept_out = MEMORY.min(size - 1);
        let recent = self.recent[occasion];
        let excluded = |index: usize| recent[..kept_out].contains(&(index as u8));
        let choices = (0..size).filter(|&index| !excluded(index)).count();
        let mut skip = self.next_random() as usize % choices;
        let mut chosen = 0;
        for index in (0..size).filter(|&index| !excluded(index)) {
            if skip == 0 {
                chosen = index;
                break;
            }
            skip -= 1;
        }
        let history = &mut self.recent[occasion];
        history.copy_within(0..MEMORY - 1, 1);
        history[0] = chosen as u8;
        chosen
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;

    #[test]
    fn the_last_three_lines_are_never_repeated() {
        let mut picker = LinePicker::new(7);
        let picks: Vec<usize> = (0..400).map(|_| picker.pick(1, 8)).collect();
        for window in picks.windows(4) {
            assert!(!window[..3].contains(&window[3]), "{window:?}");
        }
        for line in 0..8 {
            assert!(picks.contains(&line), "line {line} is reachable");
        }
    }

    #[test]
    fn small_pools_still_vary() {
        let mut picker = LinePicker::new(3);
        assert_eq!(picker.pick(2, 1), 0);
        let picks: Vec<usize> = (0..10).map(|_| picker.pick(4, 2)).collect();
        assert!(picks.windows(2).all(|pair| pair[0] != pair[1]), "a pool of two alternates: {picks:?}");
        let picks: Vec<usize> = (0..30).map(|_| picker.pick(5, 3)).collect();
        assert!(picks.windows(3).all(|run| run[0] != run[2] && run[0] != run[1]), "a pool of three cycles: {picks:?}");
    }

    #[test]
    fn occasions_keep_separate_memories() {
        let mut picker = LinePicker::new(11);
        let first = picker.pick(0, 4);
        for _ in 0..5 {
            picker.pick(1, 4);
        }
        assert_ne!(picker.pick(0, 4), first, "occasion 0 still remembers its own last pick");
    }
}
