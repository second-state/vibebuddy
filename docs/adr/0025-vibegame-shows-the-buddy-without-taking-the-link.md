---
status: accepted
---

# VibeGame shows the buddy without taking the link

On 2026-11-11 dragon decided that VibeBuddy also runs on the game handheld that runs QingOS (the Anbernic RG 34XXSP): the buddy, the task cards and the lines, on a device people hold in their hands. The protocol is in [`vibegame.md`](../vibeplay.md).

- **The VibeGame is the server, as the box is on the local network.** It accepts connections on its own port and answers mDNS under its own service type; the daemon finds it and connects. The daemon already finds, connects to and handshakes with a box this way, so most of that code serves the VibeGame too.
- **It does not take the link.** ADR-0011 stays: a computer links to one box. A VibeGame is another kind of device beside it. It gets the same snapshot as the box and can only ask for what K2 and a tap ask for. It never gets pairing, Wi-Fi, firmware, Character packs or voice. So a person with a box and a VibeGame uses both.
- **When both are there, the box speaks.** The daemon marks each event `quiet` while a box is linked, and the VibeGame then shows without playing. One announcement, one line.
- **Pairing reads a code off the screen.** The VibeGame has no cable, so the six-digit code it shows is the proof that the person is at it. The computer sends the code inside a Noise XX session, and after that they connect with Noise IK as a box does.
- **Over Wi-Fi only, local network first.** The Relay comes later; whether it stays on the Paid plan is not decided.
- **The VibeGame app lives in this repo** (`vibegame-app/`), so the protocol and both ends change in one PR. QingOS builds it into its image from a pinned commit. It is a wasm app built on QingOS's public `qapp`.
- **It is free.** People pay for the VibeGame, not for this app.

## Considered options

- **The VibeGame connects to the daemon.** The VibeGame would need to find the computer, and the daemon a new server. Rejected: the same amount of new work on the VibeGame, and none of the daemon's box code reused.
- **The VibeGame is a box.** Rejected: it would take the link, so the box and the VibeGame could not be used together, and pairing would need a cable.
- **The VibeGame app lives in QingOS's repo.** Rejected: every protocol change would need two PRs in two repos, and the app could not use this repo's GPL code and CC BY-SA assets without care.

## Consequences

- The daemon gets a second discovery, a list of paired VibeGames, and a writer per VibeGame that gets every state event. The apps get a VibeGames part on the Device page.
- QingOS gets two capabilities for apps: accepting connections on a declared port, and an mDNS answer declared in the manifest (longzhi/qingos).
- A pairing done with an attacker in the middle is not detected (see `vibegame.md`). Approving an Agent's request from the VibeGame needs a stronger pairing first.

On 2026-11-12 dragon renamed the VibeGame to VibePlay, in the code and the protocol as well: the name VibeGame was hard to find in search and other brands use it. The record above keeps the old name; [`vibeplay.md`](../vibeplay.md) has the current one.
