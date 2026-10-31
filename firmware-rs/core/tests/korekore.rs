//! The korekore colours and sprites as generated from docs/design/korekore.

use vibebuddy_firmware_core::korekore::{sprites, theme};

#[test]
fn colours_are_the_tokens_in_rgb565() {
    // #1c1730: (0x1c >> 3) << 11 | (0x17 >> 2) << 5 | 0x30 >> 3
    assert_eq!(theme::INK, 0x18A6);
    assert_eq!(theme::BOX_GROUND, theme::INK);
    assert_eq!(theme::SUN, 0xFE87);
    assert_eq!(theme::BOX_TEXT, 0xFF9B);
}

#[test]
fn every_sprite_is_whole_and_indexes_its_palette() {
    for (name, sprite) in sprites::ALL {
        assert_eq!(sprite.pixels.len(), usize::from(sprite.width) * usize::from(sprite.height), "{name}");
        assert!(sprite.pixels.iter().all(|&index| usize::from(index) <= sprite.palette.len()), "{name}");
    }
}

#[test]
fn the_box_carries_kore_the_tiles_and_the_tag_but_not_the_character_drafts() {
    let names: Vec<&str> = sprites::ALL.iter().map(|(name, _)| *name).collect();
    for expression in ["normal", "working", "input", "happy", "sad", "sleep"] {
        assert!(names.contains(&format!("kore-{expression}").as_str()), "{expression}");
    }
    assert!(names.contains(&"icon-cc") && names.contains(&"icon-nav-box") && names.contains(&"tag-your-vibebuddy"));
    assert!(!names.iter().any(|name| name.starts_with("character-")));
}

#[test]
fn kore_is_drawn_as_designed() {
    let kore = &sprites::KORE_NORMAL;
    assert_eq!((kore.width, kore.height), (20, 18));
    // Top row: the three keys on its head, then transparency.
    assert_eq!(kore.pixel(0, 0), None);
    assert_eq!(kore.pixel(4, 0), Some(theme::KORE_PINK));
    assert_eq!(kore.pixel(8, 0), Some(theme::SUN));
    assert_eq!(kore.pixel(12, 0), Some(theme::SKY));
    // The outline is Kore's own, not ink.
    assert_eq!(kore.pixel(2, 1), Some(theme::KORE_OUTLINE));
    assert_eq!(kore.pixel(20, 0), None);
}

#[test]
fn tiles_have_cut_corners() {
    let tile = &sprites::ICON_API;
    assert_eq!((tile.width, tile.height), (12, 12));
    assert_eq!(tile.pixel(0, 0), None);
    assert_eq!(tile.pixel(1, 0), Some(theme::INK));
    assert_eq!(tile.pixel(2, 1), Some(theme::AGENT_API));
}

/// The agents' own icons are the robot screens' marks (agent_icon/pixels.rs), 16 × 16: Claude Code's spark.
#[test]
fn agent_icons_are_the_agents_marks() {
    let icon = &sprites::ICON_CC;
    assert_eq!((icon.width, icon.height), (16, 16));
    assert_eq!(icon.pixel(0, 0), None);
    assert_eq!(icon.pixel(4, 0), Some(0xDBAA));
}
