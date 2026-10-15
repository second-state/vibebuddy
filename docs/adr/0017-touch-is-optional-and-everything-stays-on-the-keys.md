---
status: accepted
---

# Touch is optional, and everything stays on the keys

Boxes of the same board (ATK-DNESP32S3-BOX V1.1) differ in their screen module: one we probed has a CHSC5432 capacitive touch controller at `0x2E` on I2C0, another has nothing there ([`hardware.md`](../hardware.md), "Touch, and what else is on I2C0"). A buyer can't tell which they'll get. So the firmware probes `0x2E` at boot, and touch only adds to the keys: anything that changes state or does something can still be done with K0, K1 and K2, whose mapping (ADR-0007) doesn't change. The only touch-only thing allowed is decoration, such as the buddy's reaction when petted.

The first version knows one gesture, the tap, and uses it three ways: a tap on a menu row selects it and acts on it (K1 to the row, then K0), a tap on a key hint at the bottom of the menu is that key, and a tap on a task card takes the user to that card's agent. A tap on the buddy pets it. A tap also counts as a key press for Leisure and Boredom, which wakes a screen dimmed for the night.

## Rejected options

Making touch the main input and dropping the key hints: half the boxes would lose features. Swipes, long presses and settings that need sliders or a keyboard on the box: more to get wrong for little gain over the keys, and the settings belong in the App anyway (ADR-0007). Starting or pausing the Pomodoro by touch: a box on a desk gets brushed, and a timer that starts by accident is worse than one more key press. Falling back to K2's destination when a tapped card's agent is gone: opening something other than what was tapped is more confusing than nothing.

## Consequences

The daemon gives each item in `tasks` a short stable `id`, and the box answers a tap on a card with a `focus` event naming it; the daemon opens that activity's source the way K2 does, on macOS and Linux alike. The box reports `TOUCH CHSC5432` or `TOUCH NONE` and shows it on its STATUS view; the daemon and the Apps don't use it, since none of their behaviour depends on it. Hit testing reuses the rectangles the display draws, so a layout change moves the touch targets with it.
