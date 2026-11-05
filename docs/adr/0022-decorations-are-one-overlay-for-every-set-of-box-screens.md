---
status: accepted
---

# Decorations are one overlay for every set of box screens

On 2026-11-04 Michael asked for holiday themes the App writes to the box: background, colours, sound and a cartoon figure, starting with Thanksgiving and Christmas. The figure and the sound are a Character, and the holiday Characters Tom and Pip shipped that way (#231). What is left is the rest of the screen. The box already has two sets of box screens (ADR-0021), so a holiday look drawn per set would be designed, built and tested twice, and again for every set we add.

So a decoration is a third thing beside the Character and the box screens, and it uses only what doesn't depend on the layout:

- **Kinds.** Things that fall or drift over the whole screen (snow, leaves) and things that run along its edges (a string of lights, a frame of holly). Nothing that sits at a place a layout gives meaning to: no hats on the buddy, no new backgrounds, no recoloured cards.
- **One drawing for every set of box screens.** The box draws the decoration over the finished 320×240 frame, whichever set drew it, before the night dimming, the menu and the tap outline (`decoration.rs`). Both sets render a full frame and flush it whole (`canvas.rs`, `lcd.rs`), so there is one place to add it. A new set of box screens gets every decoration for free.
- **It never hides what the screen means.** It leaves the top 4 rows alone, where both sets draw the state colour (4 rows on the robot screens, 3 on korekore). It doesn't use the state colours (ready blue, working orange, needs input yellow, done green, failed red) as a main colour, since korekore fills the whole screen with some of them: Christmas lights are warm white and gold with small dots of other colours, not red and green bulbs. A flake or leaf that would touch anything but plain ground (text, an edge, an icon) isn't drawn in that frame, so it seems to pass behind it and text stays whole; this looks at the pixels, not at either layout. While the box needs input, it stops moving, so the only motion on the screen is the buddy asking.
- **In the firmware, picked by a command.** Like the box screens, each decoration is compiled in and switched over the link, and the box keeps the choice across restarts. Off is the default. Turning one on doesn't write the Character pack or flash anything.
- **Not tied to a Character or a date.** Pip with no snow and the robot with snow are both fine. The App doesn't turn a decoration on by the calendar; the user picks it, as with a Character.

## Considered options

- **Holiday backgrounds and colours per set of box screens.** Rejected: twice the design and the pixel tests for each holiday, and a new set of box screens would start with none.
- **Decorations carried in the Character pack.** Rejected: the pack is 2 MB and mostly audio, a decoration would then change with the Character, and the firmware would have to run drawing data it can't check at build time.
- **Drawing the decoration under the screens, as a background.** Rejected: both sets fill the whole frame with their own ground first, so a background would need changes in each set, which is what this ADR avoids.
- **Turning a decoration on by date or region.** Not now: a box that changes on its own is a surprise, and holidays differ by country. It can come later as an App setting that suggests one.

## Consequences

- The firmware gets a decoration step after each set's frame, a `device.decoration` command and a `DECORATION` line, the way `device.screens` works (docs/protocol.md). The settings record has no free word, so the decoration takes the second byte of the screens word; firmware from before it reads a decorated korekore record as the robot's, which only matters after a rollback. The robot and korekore pixels stay the same with the decoration off. `cargo run -p vibebuddy-firmware-core --example decoration_preview -- <dir>` draws each decoration on both sets.
- A moving decoration redraws the frame every 150 ms between the screens' own frames, without speeding up the buddy's animation. The rate is checked on the box. It holds still while the menu is up, while the box needs input, and while the screen is dimmed for the night.
- The Mac app and the Linux app both get a decoration picker on the Device page, off while the box is away or runs firmware that doesn't report a decoration, with zh-Hans strings on both sides.
- The first two decorations are Leaves (falling autumn leaves, for Thanksgiving 2026 on 11-26) and Snow (for Christmas). Both ship in one firmware release and one App release about a week before Thanksgiving, and that App release also carries Tom and Pip. Lights come after.
