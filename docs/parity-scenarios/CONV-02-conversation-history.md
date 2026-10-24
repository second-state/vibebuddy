# CONV-02: Read and follow the voice commands and the buddy's replies

Category: Conversation tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/ConversationView.swift` (`follow`, `reload`, `content`, `ExchangeRow`)
- `app/Sources/VibeBuddy/DaemonClient.swift` (`resident`, `residentHistory`)

Daemon API: `GET /v1/resident (every 2 s)`; `GET /v1/resident/history?limit=20 (60 s timeout)`

## Initial state (input)

- The Conversation tab is shown and voice commands are ready (no `problem`).

## Trigger

The user opens the tab, or a voice command ends on the box while it is open.

## Workflow (Mac app)

1. The header: "Your last voice commands and the buddy's spoken replies, newest at the bottom." with Refresh and a spinner while reading.
2. On opening, the app reads the info and the last 20 exchanges. While the tab stays open it asks the info every 2 s, and reads the history again only when its `revision` moved, or when voice has just become ready. A daemon without `revision` gets its history read every 5 s.
3. Each exchange shows what the user said on the right (accent colour) and the buddy's replies on the left; the view scrolls to the newest. A new thread (the other Agent answering) gets a fresh scroll position.
4. Empty: "Nothing said yet" with "Hold K0 on the box and talk to the buddy. What you say and what it says back show up here."
5. Daemon unreachable: "Couldn't read the conversation" with "The daemon isn't running. Restart it on the Advanced tab."; once it answers again the history is read anew, since a restarted daemon counts from 0.
6. Another failure: "Couldn't read the conversation" with "The buddy's session isn't available: {reason}".
7. Leaving the tab or closing the window stops the polling; the next visit reads again.

## Final state (output)

- The tab mirrors the conversation held on the box.

## Check on Linux

- [ ] Same polling rule, empty, daemon-down and error states, and newest-at-the-bottom layout?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
