---
status: accepted
---

# The Relay is a closed-source Cloudflare service; pairing, not accounts, decides who may link

We want a box to work with a computer on another network, and we want a place for later online features (paid features, Agents, boxes talking to each other). We decided to add the Relay: a service that carries a link when the computer and the box can't reach each other directly. A link prefers USB, then the local network, then the Relay; moving between them is not a drop. The Relay is optional: a box works fully over USB or the local network without it.

- **Built on Cloudflare Workers with one Durable Object per box**, on our own domain beside the update manifest (ADR-0010). The Durable Object is the single place that decides which computer a box is linked to (ADR-0011).
- **Closed source.** The App and firmware side of the protocol stay in this repo.
- **TLS only, no end-to-end encryption.** The Relay can read the protocol messages, including task card titles. The hook's privacy filter is unchanged, and the README must say what the Relay can see.
- **No accounts.** A computer may link to a box only after pairing with it. Pairing is done over USB in the first version (physical presence is the authorization) and exchanges keys that authenticate the local network and the Relay alike, replacing the separate LAN token of #3. Each box makes its own key pair the first time new firmware boots; nothing is provisioned at the factory. Accounts can be layered on pairing later.
- **A box with Wi-Fi set up stays connected to the Relay even while linked over USB**, so another computer can see it and take it over.

## Considered options

- **Local network only.** Enough to take the box off the cable, but not for a computer on another network, and no foundation for online features.
- **End-to-end encryption, Relay sees ciphertext.** Rejected for simplicity; the data was judged not sensitive.
- **Accounts from day one.** Adds sign-up before anyone needs it.
- **Bluetooth pairing.** The other end is a Mac or Linux computer, not a phone, and USB is always there (a battery box still charges over USB-C). Pairing is designed independent of transport, so an on-screen code confirmed with a button, or ESP32-style BLE provisioning once there is a phone app, can come later.
- **A Rust service on our own server.** Same language as the daemon, but we'd run availability and scaling ourselves.

## Consequences

Partly supersedes ADR-0009's stance that the open-source tool shouldn't depend on a VibeBuddy service: it still doesn't, since the Relay is optional, but the project now runs one. Paid generation through a server, which ADR-0009 rejected, would need its own ADR.
