# Vendored crates

## iced_tiny_skia 0.14.1

iced's software renderer, from crates.io, with two changes:

- `src/window/compositor.rs`: `present` always draws the whole frame.
- `src/engine.rs`: a quad's shadow is clipped like the quad.

Upstream draws only the damage when the window system gives back a buffer that still holds an earlier frame (its age is more than 0). Wayland does; macOS doesn't. On Wayland (Omarchy, Hyprland, scale 2) the damage missed parts of the korekore pages, so the window showed text and cards from no frame at all: a Character card with its art but no name, the Advanced daemon card with no text or button. With the whole frame drawn each time, the window matches the off-screen snapshots.

Upstream also draws a quad's shadow with no clip mask, so the shadow of a card cut off by a scrollable's edge was drawn past that edge, as a dark bar over the bottom of the Character and General pages.

The workspace `Cargo.toml` puts it in place of the crates.io one with `[patch.crates-io]`. When iced is updated, take the new version's source, make the same changes, and check the korekore pages on a Wayland desktop. Drop the copy once upstream does both itself.

License: MIT, as iced (https://github.com/iced-rs/iced).
