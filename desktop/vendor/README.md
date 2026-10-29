# Vendored crates

## iced_tiny_skia 0.14.1

iced's software renderer, from crates.io, with one change in `src/window/compositor.rs`: `present` always draws the whole frame.

Upstream draws only the damage when the window system gives back a buffer that still holds an earlier frame (its age is more than 0). Wayland does; macOS doesn't. On Wayland (Omarchy, Hyprland, scale 2) the damage missed parts of the korekore pages, so the window showed text and cards from no frame at all: a Character card with its art but no name, the Advanced daemon card with no text or button, a dark bar over the bottom of a page. With the whole frame drawn each time, the window matches the off-screen snapshots.

The workspace `Cargo.toml` puts it in place of the crates.io one with `[patch.crates-io]`. When iced is updated, take the new version's source, make the same change, and check the korekore pages on a Wayland desktop. Drop the copy once upstream draws correctly from an aged buffer.

License: MIT, as iced (https://github.com/iced-rs/iced).
