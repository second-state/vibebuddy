---
status: accepted
---

# The box's own settings live in a menu behind a long press on K1

Four of the box's seven key gestures were hidden long presses or a chord, and the box couldn't show or change its own volume. Following Muse's firmware on the same box, we added a menu that is stepped through with short presses only: K1 moves down, K0 acts on the row (values cycle), K2 closes, and it closes itself after 30 s. It opens on a long K1, which used to start Leisure; Leisure now only starts on its own. The frequent actions keep their single press (K0 Pomodoro, K1 duty/Pomodoro, K2 back to the agent's window), and the other long presses and the K1 + K2 chord stay as shortcuts to rows in the menu. See [`device-menu.md`](../device-menu.md).

## Rejected options

Muse's two-key layout, with K1's short press opening the menu: K1's one-press switch between duty and Pomodoro is used often, and K2's jump back to the agent is the box's most used action, so neither could move into a menu. Opening the menu with K0 long or K2 long instead: those give up a Pomodoro phase and mute, both of which want to stay one gesture away. Moving the app's settings onto the box: connecting agents, the voice library, firmware updates and notifications need the Mac's files, network or system settings, so the menu holds only what the box owns.

## Consequences

Leisure can no longer be started on demand; demos use the time-compressed build. The C fallback firmware keeps the old long K1. A volume change from the box prints the same `VOLUME <n>` line as `device.volume`, which the daemon already applies, so the app's slider follows without a protocol change.
