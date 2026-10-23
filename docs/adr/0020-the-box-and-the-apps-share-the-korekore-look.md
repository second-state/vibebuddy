---
status: proposed
---

# The box and the apps share the korekore look

VibeBuddy gets a face: **korekore** (これこれ, "here, here!"), a kawaii, Japanese-cartoon desk cheerleader, with **your vibebuddy** kept beside the name so people who already bought a box recognise it. The design lives in [`docs/design/korekore/`](../design/korekore/README.md): tokens, exact pixel art, and a layout spec for every screen.

Until now the box and the apps looked like two products. The box was a dark retro screen with a code-drawn robot; the apps were styled like System Settings on purpose, with branding only in onboarding, About and the menu bar icon ([`app.md`](../app.md)). That kept the apps out of the way, but nothing on the Mac said this was the same friend that sits on the desk. So korekore gives the box and both apps one look: cream ground, Ink outlines, solid sticker shadows, a rounded face for what people read, pixel type for the box and the machine-ish bits, and pixel art for the buddy and the agents.

**korekore is built as a new UI beside the current one, not over it.** The current settings window, onboarding, menu bar item and box screens stay exactly as they are and stay the default. The korekore UI lives in new files, behind a switch (an app setting and a firmware build feature), until the team decides to make it the default; only then are the old views removed and the docs that describe them rewritten.

What the korekore UI does:

- **The settings window** has a left sidebar instead of a top tab bar. The sidebar's head is a mini box that redraws the box's current screen from the daemon's status (not a screen capture, so it costs nothing over the UART bridge), then the six sections with pixel tiles. Every page uses the system's cards, buttons, toggles and pixel state tags.
- **The default Character** is **Kore**, the box itself drawn as a 20 × 18 sprite with the three keys on its head. The robot stays selectable. Other Characters move to one shared chibi style; production looks keep the 48 × 64, 16-colour, four-frame spec ([`characters.md`](../characters.md)).
- **Agents** are shown by 12 × 12 pixel tiles instead of `CC:` / `CX:` / `CI:` prefixes, on the box and in the apps.
- **On the box** it keeps the state language ([`pet.md`](../pet.md)) but says it louder where it should: needs input takes over the whole screen in `sun`, done in `mint` for three seconds, Pomodoro in `kore-pink`; failed stays on Ink with a peach border; no link turns the screen gray.
- **The menu bar** item is Kore, and its menu is a small panel (`MenuBarExtra` with the window style on the Mac).

## Considered options

- **Restyle the current UI in place.** Fastest to see, but it puts finished, shipped screens at risk while the new look is still being tried. Rejected in favour of building beside it.
- **Keep the native settings window and add an accent colour and a mascot strip.** Kept the System Settings rule. Rejected: the app still felt like a different product from the box.
- **Restyle the top tabs as pills.** Tried in the mockups; the window read as a web page with tabs. The sidebar gives the box a permanent place on screen instead.

## Consequences

- Nothing about the current UI changes, so its tests, screenshots, the Rust/C screen comparison (`tools/compare-display.sh`) and the Omarchy theming on Linux are untouched. The korekore UI brings its own tests and previews.
- The Mac and Linux korekore UIs are built together, as AGENTS.md requires for anything a user sees.
- The korekore UI uses its own palette everywhere, Omarchy included; the current Linux UI keeps following Omarchy.
- Fonts follow the design exactly: both apps bundle M PLUS Rounded 1c and DotGothic16 (SIL OFL 1.1), and the korekore box firmware carries a DotGothic16 glyph table, about 115 KB of flash for the same character set as today's 12 px CJK table.
- Text colours were checked for 4.5:1; the state fills (`sun`, `mint`, `sky`) are never used as text, which is why the system has separate `*-text` tokens.
- The repository, bundle identifiers, config paths and the `vibebuddy` command names do not change. Renaming the product in code is a separate decision.
