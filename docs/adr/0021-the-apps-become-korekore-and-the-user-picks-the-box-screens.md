---
status: accepted
---

# The apps become korekore, and the user picks the box's screens

[ADR-0020](0020-the-box-and-the-apps-share-the-korekore-look.md) built korekore as a second UI beside the current one, behind a hidden switch, with the box's korekore screens in a separate firmware build, and left the rest for later. Reviewing the handoff on 2026-10-23, we settled what "later" is. This ADR replaces the parts of ADR-0020 it names; the rest stands.

- **The apps have one style, korekore.** There is no setting to pick the apps' style. While the korekore apps are being built (Phase 2 of [`docs/design/korekore/`](../design/korekore/README.md)) the current settings window, onboarding and menu bar stay what users get, and the korekore UI opens only with the development switch `VIBEBUDDY_UI=korekore`. Once all of Phase 2 has landed and been accepted, korekore replaces the current app UI in one step, and the old views and the docs that describe them go. Phase 2 was accepted on 2026-10-30: the switch and the old views are removed, and both apps open korekore.
- **The user picks the box's screens** in the korekore app: the current robot screens or the korekore screens. The robot screens stay the default. (This is not a Character's look: either set of screens can show any Character.)
- **One firmware image carries both.** Changing the box's screens is a command over the link, not a flash: no second image to release, and no update that quietly puts a box back on the other screens. The cargo feature `korekore` from ADR-0020 is dropped. The default image is no longer byte-for-byte the same as before, but the robot screens must stay pixel-for-pixel the same (`tools/compare-display.sh` and the pinned pixels in `firmware-rs/core/tests/firmware.rs`), and the image must still fit the 4 MB OTA slot.
- **The default Character stays the robot**, with either set of screens. Kore becomes a full Character of its own (persona, voice, lines, look), using the defaults the design README gives (the Jessica voice, English with a little Japanese), in its own change.

## Considered options

- **A setting for the apps' style too.** Rejected: one style in the apps is less to build, test and explain, and the korekore style is the one we want people to see.
- **One firmware image per set of screens, flashed when the user switches.** Rejected: switching would take a minute or more over the Relay and could fail halfway, each release would ship two images, and every update would have to remember which one a box runs.
- **Keep the hidden switch and decide later.** Rejected: the decision is made; a switch that only developers can reach would keep two app UIs alive for no one.

## Consequences

- Until Phase 2 is accepted nothing users see changes; after it the current app UI is removed rather than kept beside korekore.
- The box needs a way to be told which screens to draw and to keep it across restarts, and the apps a place to choose it; both arrive with the korekore box (Phase 3).
- Every firmware image gets the korekore renderer, its sprites and the DotGothic16 glyph table (about 115 KB), so the image size is checked against the OTA slot when Phase 3 lands.
