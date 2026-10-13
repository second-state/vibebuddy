---
status: proposed
---

# Accounts belong to people, are signed into on the computer, and lend the box short voice tokens

Voice commands (ADR-0017) cost us money on every use, so they are offered only to signed-in users, and accounts have to exist first. We decided an account belongs to a person, never to a box. The user signs in on the computer, in the App, through the system browser, with an email one-time code or GitHub. The daemon keeps the session (`config_dir()/account`, mode 0600, beside `identity`), and the box never signs in itself. Over USB or the local network, the box's audio goes over the link to the daemon, which calls the voice service with the account session and sends the spoken reply back the same way. Over the Relay, where the computer is elsewhere, audio never crosses the Relay: the daemon fetches a voice token that lasts an hour for the box it is linked to and sends it over the link, and the box presents that token to the voice service directly over Wi-Fi, for recognition and for the spoken reply's audio. Accounts are layered on pairing and change nothing about it: linking, takeover and the Relay (ADR-0011, ADR-0012) need no account, and an account doesn't pair anything.

- **Voice is free, with a daily allowance per account** (about 100 commands, set by the service). The service also has a global daily spending cap: past it, voice stops for everyone for the day and we are alerted. Payment can be layered on the account later if real costs call for it.
- **An account keeps no list of computers or boxes.** A token names the account; the box key rides along only as a label for logs.
- **We store** the sign-in identity (an email address or a GitHub user id), when the account was made, daily command counts and daily cost totals; never audio, transcripts or replies. Users can delete their account from the App.
- **The service** is a new Worker, `vibebuddy-api` at `api.korekore.ai`, holding accounts and voice together, with accounts in D1, in the closed `vibebuddy-cloud` repo beside the Relay but separate from it. Its API is documented in this repo in `docs/account-protocol.md`, as the Relay's is, and an App built from source may use it like any other.

## Considered options

- **The account belongs to the box.** A box's key is made by the box itself, changes with NEW ID, sits in unencrypted flash and is backed by no hardware identity, so it can't be who pays or who is limited. A box account would also not survive a box changing hands sensibly.
- **The box signs in itself**, showing a code to enter on another device. Leaves a lasting account credential on a box without a keyboard or encrypted storage, while voice needs a signed-in computer anyway, since the resident session runs there.
- **Audio always goes over the link and the daemon calls the voice service.** One path and no token on the box, but over the Relay the audio would travel from the box through the cloud to the computer and back to the cloud, on the computer's upload, adding a round trip to every command. (The Relay couldn't hear it: links off the cable are end-to-end encrypted, ADR-0015.)
- **The box always calls the voice service directly.** One path, but voice would need Wi-Fi even with the box on the cable, and the daemon, which is right there, would be bypassed.
- **The Relay requires sign-in too.** Would take away something users of 0.5.0 already have without an account, to guard a service that costs about a cent per box a month.
- **Paid from day one, or free voice for buyers of a box.** A heavy user costs us about 10 dollars a month and most well under one (checked 2026-10-13: about $0.003 a command in English, about ¥0.01 in Chinese); payments, invoices, refunds and tax in two markets would cost more than that. Free voice for buyers would need orders matched to accounts by email, with no box identity to check.
- **Sign in with ChatGPT.** Codex already needs it, but it would tie our accounts to one Agent's vendor.
- **The account lists its computers and boxes.** The daemon remembers one box and a box can change its key, so the list would be unreliable; it waits for a real multi-box need.
- **Accounts inside the Relay's Worker.** Later online features are not the Relay, and the Relay has real users who shouldn't share deploys with new features.

## Consequences

- The voice service accepts two credentials: the account session from a daemon and a voice token from a box.
- The VibeBuddy Protocol gets messages carrying audio and the spoken reply between the box and the daemon, and one carrying the voice token from the daemon to the box; a short token fits a line.
- Voice over the Relay needs the box on Wi-Fi with the voice service reachable; voice over USB needs no Wi-Fi.
- Self-made box keys no longer matter for cost: what is limited is accounts, and the global cap bounds the worst case. Blocking disposable email domains, or a lower allowance for new accounts, is added only if accounts are farmed.
- What the box says when its computer isn't signed in, the allowance is used up or the global cap is hit is left to voice's design (#132).
- Signing out stops voice within the hour a token lasts.
- With ADR-0017, this is the server-side generation that ADR-0009 and ADR-0012 said would need its own decision; the App itself still calls no model.
