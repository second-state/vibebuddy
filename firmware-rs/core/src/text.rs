//! Small stack-allocated strings: lines on screen and lines on the serial port are built with it, off the heap.

use core::fmt;

pub struct Text<const N: usize> {
    bytes: [u8; N],
    length: usize,
}

impl<const N: usize> Text<N> {
    pub const fn new() -> Self {
        Self { bytes: [0; N], length: 0 }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.length]
    }

    pub fn len(&self) -> usize {
        self.length
    }

    pub fn is_empty(&self) -> bool {
        self.length == 0
    }

    /// Takes as much as fits and truncates the rest, like snprintf.
    pub fn push_bytes(&mut self, bytes: &[u8]) {
        let take = bytes.len().min(N - self.length);
        self.bytes[self.length..self.length + take].copy_from_slice(&bytes[..take]);
        self.length += take;
    }
}

impl<const N: usize> Default for Text<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> fmt::Write for Text<N> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        self.push_bytes(text.as_bytes());
        Ok(())
    }
}

/// A stack version of `format!`: `text!(24, "{} FOCUS", n)`.
#[macro_export]
macro_rules! text {
    ($size:expr, $($arg:tt)*) => {{
        let mut text = $crate::text::Text::<$size>::new();
        let _ = core::fmt::Write::write_fmt(&mut text, format_args!($($arg)*));
        text
    }};
}

/// Copies bytes truncated to at most `limit` bytes, matching C's strncpy into a fixed-size array. A cut never
/// falls inside a UTF-8 character: the partial character goes too.
pub fn truncated(bytes: &[u8], limit: usize) -> alloc::vec::Vec<u8> {
    let mut end = bytes.iter().take(limit).position(|&byte| byte == 0).unwrap_or(bytes.len().min(limit));
    while end > 0 && bytes.get(end).is_some_and(|&byte| byte & 0xc0 == 0x80) {
        end -= 1;
    }
    bytes[..end].to_vec()
}

#[cfg(test)]
mod tests {
    use super::truncated;

    #[test]
    fn truncation_keeps_whole_characters() {
        assert_eq!(truncated(b"abcdef", 4), b"abcd");
        assert_eq!(truncated(b"ab\0cd", 4), b"ab");
        assert_eq!(truncated("你好".as_bytes(), 4), "你".as_bytes());
        assert_eq!(truncated("你好".as_bytes(), 6), "你好".as_bytes());
    }
}
