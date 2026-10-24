---
status: proposed
---

# Accounts belong to people and are signed into on the computer; the box never holds a credential

Voice commands (ADR-0017) cost us money on every use, so they are offered only to signed-in users, and accounts have to exist first. We decided an account belongs to a person, never to a box. The user signs in on the computer, in the App, through the system browser, through GitHub first; an email one-time code joins later (see Consequences). The daemon keeps the session (`config_dir()/account`, mode 0600, beside `identity`), and the box never signs in itself. However the box is linked, over USB, the local network or the Relay, its audio goes over the link to the daemon, which calls the voice service with the account session and sends the spoken reply's audio back the same way; the box never calls the service and holds no credential. (Until 2026-10-24 the box was to call the service itself over the Relay with a voice token; see Considered options.) Accounts are layered on pairing and change nothing about it: linking, takeover and the Relay (ADR-0011, ADR-0012) need no account, and an account doesn't pair anything.

- **Voice is free, with a daily allowance per account** (about 100 commands, set by the service). The service also has a global daily spending cap: past it, voice stops for everyone for the day and we are alerted. Payment can be layered on the account later if real costs call for it.
- **An account keeps no list of computers or boxes.**
- **We store** the sign-in identity (an email address or a GitHub user id), when the account was made, daily command counts and daily cost totals; never audio, transcripts or replies. Users can delete their account from the App.
- **The service** is a new Worker, `vibebuddy-api` at `api.korekore.ai`, holding accounts and voice together, with accounts in D1, in the closed `vibebuddy-cloud` repo beside the Relay but separate from it. Its API is documented in this repo in `docs/api.md`, as the Relay's is in `docs/relay-protocol.md`, and an App built from source may use it like any other.

## Considered options

- **The account belongs to the box.** A box's key is made by the box itself, changes with NEW ID, sits in unencrypted flash and is backed by no hardware identity, so it can't be who pays or who is limited. A box account would also not survive a box changing hands sensibly.
- **The box signs in itself**, showing a code to enter on another device. Leaves a lasting account credential on a box without a keyboard or encrypted storage, while voice needs a signed-in computer anyway, since the resident session runs there.
- **Over the Relay, the box calls the voice service itself with a voice token** the daemon fetches for it (an hour long, signed by the service, sent over the link). This was the first decision, and the service and daemon sides were built (vibebuddy-cloud#15, #172) before it was dropped on 2026-10-24. It saves one hop, box to cloud to computer and back to the cloud, which costs a few hundred milliseconds for a few seconds of audio against an answer that takes about ten seconds. It costs much more: an HTTP client and a second TLS session in the firmware, on RAM that is already tight; a token endpoint and a token handout that exist for this path alone; and a box that must follow every change to the voice API, while firmware reaches users far more slowly than the daemon. Its other reason, that the Relay would carry the audio, went with ADR-0015: links off the cable are end-to-end encrypted, so the Relay carries only ciphertext.
- **The box always calls the voice service directly.** One path, but voice would need Wi-Fi even with the box on the cable, and the daemon, which is right there, would be bypassed.
- **The Relay requires sign-in too.** Would take away something users of 0.5.0 already have without an account, to guard a service that costs about a cent per box a month.
- **Paid from day one, or free voice for buyers of a box.** A heavy user costs us about 10 dollars a month and most well under one (checked 2026-10-13: about $0.003 a command in English, about ¥0.01 in Chinese); payments, invoices, refunds and tax in two markets would cost more than that. Free voice for buyers would need orders matched to accounts by email, with no box identity to check.
- **Sign in with ChatGPT.** Codex already needs it, but it would tie our accounts to one Agent's vendor.
- **The account lists its computers and boxes.** The daemon remembers one box and a box can change its key, so the list would be unreliable; it waits for a real multi-box need.
- **Accounts inside the Relay's Worker.** Later online features are not the Relay, and the Relay has real users who shouldn't share deploys with new features.

## Consequences

- The voice service takes one credential, the account session, and only daemons hold it.
- The VibeBuddy Protocol carries the audio and the spoken reply between the box and the daemon over every kind of link. Over the Relay that is about 21 lines and 16 KB a second of recording, inside the Relay's 50 messages a second and 8 KiB a message, and the reply's MP3 at about 6 KB a second.
- A user may set their own speech recognition service in the App. The daemon calls it with the user's key, which never reaches us, and only the spoken reply comes from our service.
- Voice over USB needs no Wi-Fi.
- Self-made box keys no longer matter for cost: what is limited is accounts, and the global cap bounds the worst case. Blocking disposable email domains, or a lower allowance for new accounts, is added only if accounts are farmed.
- What the box says when its computer isn't signed in, the allowance is used up or the global cap is hit is left to voice's design (#132).
- Signing out stops voice at once.
- Email sign-in is deferred (2026-10-23): Cloudflare sends mail to arbitrary addresses only on the Workers Paid plan. GitHub suits a product for programmers; the email code is built and waits behind the mail setup (vibebuddy-cloud#4).
- With ADR-0017, this is the server-side generation that ADR-0009 and ADR-0012 said would need its own decision; the App itself still calls no model.
