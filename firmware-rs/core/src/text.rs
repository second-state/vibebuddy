//! 栈上的小字符串：画面上的一行、串口上的一行都用它拼，不上堆。

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

    /// 放得下多少放多少，超出的截掉，和 snprintf 一样。
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

/// `format!` 的栈上版本：`text!(24, "{} FOCUS", n)`。
#[macro_export]
macro_rules! text {
    ($size:expr, $($arg:tt)*) => {{
        let mut text = $crate::text::Text::<$size>::new();
        let _ = core::fmt::Write::write_fmt(&mut text, format_args!($($arg)*));
        text
    }};
}

/// 按字节截断复制，最多 `limit` 字节，与 C 的 strncpy 到定长数组一致。
pub fn truncated(bytes: &[u8], limit: usize) -> alloc::vec::Vec<u8> {
    let end = bytes.iter().take(limit).position(|&byte| byte == 0).unwrap_or(bytes.len().min(limit));
    bytes[..end].to_vec()
}
