//! UI strings are keyed in English, written exactly as in the Mac app's table (`%@`, `%lld`), so both apps share
//! `app/Localization/zh-Hans.lproj/Localizable.strings` and `tools/check-localization.py` checks this app too.
//! Chinese is shown when the language picked in Settings, or else the locale, asks for it; everything else gets the
//! English key.

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::OnceLock;

const TABLE: &str = include_str!("../../app/Localization/zh-Hans.lproj/Localizable.strings");

/// Looks up `key` and fills its `%@` / `%lld` slots, positional ones (`%2$@`) included, from `args` in order.
pub fn tr(key: &str, args: &[&dyn Display]) -> String {
    let text = chinese()
        .and_then(|table| table.get(key))
        .map(String::as_str)
        .unwrap_or(key);
    format(text, args)
}

fn chinese() -> Option<&'static HashMap<String, String>> {
    static TABLE_CELL: OnceLock<Option<HashMap<String, String>>> = OnceLock::new();
    TABLE_CELL
        .get_or_init(|| wants_chinese().then(|| parse(TABLE)))
        .as_ref()
}

/// Whether the UI speaks Chinese, which also decides which voices are listed first.
pub fn is_chinese() -> bool {
    chinese().is_some()
}

fn wants_chinese() -> bool {
    UiLanguage::saved().chinese()
}

/// The UI language picked in Settings. It is read once at start, so a change applies after a restart, as on the Mac.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiLanguage {
    System,
    English,
    Chinese,
}

impl UiLanguage {
    pub const ALL: [Self; 3] = [Self::System, Self::English, Self::Chinese];

    fn file() -> Option<std::path::PathBuf> {
        crate::config_dir().map(|dir| dir.join("language"))
    }

    pub fn saved() -> Self {
        match Self::file().and_then(|file| std::fs::read_to_string(file).ok()).as_deref().map(str::trim) {
            Some("en") => Self::English,
            Some("zh-Hans") => Self::Chinese,
            _ => Self::System,
        }
    }

    /// "System" removes the file, so the locale decides again.
    pub fn save(self) -> Result<(), String> {
        let file = Self::file().ok_or("HOME is not set")?;
        let result = match self {
            Self::System => std::fs::remove_file(&file).or_else(|error| match error.kind() {
                std::io::ErrorKind::NotFound => Ok(()),
                _ => Err(error),
            }),
            Self::English | Self::Chinese => {
                let code = if self == Self::Chinese { "zh-Hans" } else { "en" };
                file.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| std::fs::write(&file, code))
            }
        };
        result.map_err(|error| format!("{}: {error}", file.display()))
    }

    /// Whether the UI is shown in Chinese once this choice applies.
    pub fn chinese(self) -> bool {
        match self {
            Self::System => locale_wants_chinese(),
            Self::English => false,
            Self::Chinese => true,
        }
    }
}

impl Display for UiLanguage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The languages are named in themselves, so they can be found whichever one the UI is in.
        match self {
            Self::System => f.write_str(&tr("System", &[])),
            Self::English => f.write_str("English"),
            Self::Chinese => f.write_str("简体中文"),
        }
    }
}

/// The first of the POSIX locale variables that is set decides, as it does for every other program.
fn locale_wants_chinese() -> bool {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
        .is_some_and(|locale| locale.starts_with("zh"))
}

/// `"key" = "value";` lines; comments and anything else are skipped.
fn parse(table: &str) -> HashMap<String, String> {
    let mut entries = HashMap::new();
    for line in table.lines() {
        let Some((key, rest)) = literal(line.trim_start()) else { continue };
        let Some(rest) = rest.trim_start().strip_prefix('=') else { continue };
        if let Some((value, _)) = literal(rest.trim_start()) {
            entries.insert(key, value);
        }
    }
    entries
}

/// A double-quoted literal at the start of `text`, unescaped, and what follows it.
fn literal(text: &str) -> Option<(String, &str)> {
    let mut chars = text.strip_prefix('"')?.char_indices();
    let mut out = String::new();
    while let Some((index, ch)) = chars.next() {
        match ch {
            '"' => return Some((out, &text[index + 2..])),
            '\\' => match chars.next()?.1 {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                other => out.push(other),
            },
            ch => out.push(ch),
        }
    }
    None
}

fn format(text: &str, args: &[&dyn Display]) -> String {
    let mut out = String::new();
    let mut rest = text;
    let mut next = 0;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let spec = &rest[start + 1..];
        let digits = spec.bytes().take_while(u8::is_ascii_digit).count();
        let (index, after_index) = match spec[digits..].strip_prefix('$') {
            Some(after) if digits > 0 => (spec[..digits].parse::<usize>().ok().map(|n| n - 1), after),
            _ => (None, spec),
        };
        let Some(length) = ["lld", "@", "d"].iter().find(|kind| after_index.starts_with(**kind)).map(|kind| kind.len())
        else {
            out.push('%');
            rest = spec;
            continue;
        };
        let slot = index.unwrap_or_else(|| {
            next += 1;
            next - 1
        });
        if let Some(arg) = args.get(slot) {
            out.push_str(&arg.to_string());
        }
        rest = &after_index[length..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_fill_in_order_or_by_position() {
        assert_eq!(format("Mode: %@", &[&"On duty"]), "Mode: On duty");
        assert_eq!(format("%lld h %lld min", &[&1, &2]), "1 h 2 min");
        assert_eq!(format("%2$@ before %1$@", &[&"a", &"b"]), "b before a");
        assert_eq!(format("100% sure", &[]), "100% sure");
    }

    #[test]
    fn the_shared_table_parses() {
        let table = parse(TABLE);
        assert_eq!(table.get("Box not found").map(String::as_str), Some("未找到盒子"));
        assert!(table.len() > 100);
    }

    #[test]
    fn escapes_survive() {
        let table = parse(r#""a \"b\"\nc" = "x\\y"; /* note */"#);
        assert_eq!(table.get("a \"b\"\nc").map(String::as_str), Some("x\\y"));
    }
}
