//! Colors follow the Omarchy theme when there is one: Omarchy keeps the current palette in `colors.toml`
//! and rewrites it on every theme switch, so watching that one file keeps the window in step.
//! Elsewhere the app uses iced's own dark theme.

use std::path::PathBuf;
use std::time::SystemTime;

use iced::Color;
use iced::theme::{Palette, Theme};

pub fn colors_file() -> Option<PathBuf> {
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))?;
    Some(state.join("omarchy/current/theme/colors.toml"))
}

/// The file's modification time, so callers reload only when it changed.
pub fn stamp() -> Option<SystemTime> {
    std::fs::metadata(colors_file()?).and_then(|meta| meta.modified()).ok()
}

pub fn load() -> Theme {
    colors_file()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| from_colors(&text))
        .unwrap_or(Theme::Dark)
}

/// Builds a theme from Omarchy's `key = "#rrggbb"` lines, falling back along the same aliases Omarchy uses
/// (`accent` is `blue` is `color4`, and so on) so partial themes still work.
fn from_colors(text: &str) -> Option<Theme> {
    let value = |key: &str| {
        text.lines().find_map(|line| {
            let (name, value) = line.split_once('=')?;
            (name.trim() == key).then(|| value.trim().trim_matches('"').to_owned())
        })
    };
    let color = |keys: &[&str]| keys.iter().find_map(|key| value(key).and_then(|text| hex(&text)));
    let palette = Palette {
        background: color(&["background", "color0"])?,
        text: color(&["foreground", "color7", "color15"])?,
        primary: color(&["accent", "blue", "color4"])?,
        success: color(&["green", "color2"])?,
        warning: color(&["yellow", "color3"])?,
        danger: color(&["red", "color1"])?,
    };
    Some(Theme::custom("Omarchy", palette))
}

/// `#rrggbb`, the only form Omarchy writes.
fn hex(text: &str) -> Option<Color> {
    let digits = text.strip_prefix('#')?;
    if digits.len() != 6 {
        return None;
    }
    let channel = |at: usize| u8::from_str_radix(digits.get(at..at + 2)?, 16).ok();
    Some(Color::from_rgb8(channel(0)?, channel(2)?, channel(4)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_omarchy_palette_maps_onto_iced() {
        let theme = from_colors(
            "mode = \"light\"\naccent = \"#56949f\"\nbackground = \"#faf4ed\"\nforeground = \"#575279\"\n\
             red = \"#b4637a\"\nyellow = \"#ea9d34\"\ngreen = \"#286983\"\n",
        )
        .expect("theme");
        assert_eq!(theme.palette().primary, hex("#56949f").expect("color"));
        assert_eq!(theme.palette().background, hex("#faf4ed").expect("color"));
    }

    #[test]
    fn terminal_colors_stand_in_for_missing_names() {
        let theme = from_colors(
            "color0 = \"#000000\"\ncolor1 = \"#ff0000\"\ncolor2 = \"#00ff00\"\ncolor3 = \"#ffff00\"\n\
             color4 = \"#0000ff\"\ncolor7 = \"#cccccc\"\n",
        )
        .expect("theme");
        assert_eq!(theme.palette().primary, hex("#0000ff").expect("color"));
    }

    #[test]
    fn only_six_digit_hex_is_a_color() {
        assert_eq!(hex("#ff8000"), Some(Color::from_rgb8(255, 128, 0)));
        assert_eq!(hex("ff8000"), None);
        assert_eq!(hex("#fff"), None);
        assert_eq!(hex("#gg0000"), None);
    }

    #[test]
    fn an_unusable_file_gives_no_theme() {
        assert!(from_colors("accent = \"#56949f\"").is_none());
    }
}
