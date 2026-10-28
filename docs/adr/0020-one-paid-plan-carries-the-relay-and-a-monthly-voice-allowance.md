---
status: proposed
---

# One paid plan carries the Relay and a monthly voice allowance; we charge only for what runs on our servers

The Relay and our voices cost us money to run, and users now want to make their own Characters, which means synthesizing many lines at once. We decided to sell one Paid plan, a monthly Stripe subscription on the account (ADR-0018). It has two things: the Relay, and a monthly allowance of our voices. There is no separate purchase of credits. Without the plan an account is Free, and Free is a full product: everything over USB and the local network, voice commands and spoken replies within a small daily allowance, and making Characters with the user's own key. When a plan ends, the account goes back to Free. Nothing the user already has is taken away: Characters already made stay on the computer and the box.

- **We charge only for what runs on our servers: the Relay and our voices.** The App and the daemon are open source, so a check on the computer is a line anyone can delete. Making a Character is open to everyone. With the user's own key it costs us nothing; with our voices it uses the monthly allowance.
- **The Relay carries a link only for an account on the Paid plan.** Pairing still decides who may link (ADR-0012). The plan decides only whether the Relay carries the link. The daemon presents its account session when it connects to the Relay, and the Relay asks `vibebuddy-api` about the plan. The box needs no account, as before (ADR-0018).
- **The monthly allowance is counted in characters of our voices, per Stripe billing period.** Spoken replies and making Characters both use it. Recognition does not, because it costs little; voice commands keep a daily allowance for everyone, and the plan's is larger. Past the monthly allowance, the account gets the Free daily allowance until the next period. The price and every amount are service settings, not code.
- **The user's own key stays on the computer.** The user can enter a key for a speech provider in the App. The daemon keeps it in `config_dir()`, mode 0600, beside the account, and calls the provider itself. The key never reaches us, and nothing it speaks counts against an allowance. This is the same as the user's own recognition service in ADR-0018.
- **Making a Character uses no language model.** Its lines start as a preset Character's lines, and the user can edit the text. Its voice comes from a list of voices we choose. The look is imported, as in ADR-0009. The daemon synthesizes each line, with the user's own key or with our voices, converts it to the pack's ADPCM (ADR-0008) and builds the Character pack.
- **Payment happens on the website, never in the App.** The App opens Stripe Checkout in the system browser for the signed-in account. A Stripe webhook tells `vibebuddy-api`, which records the plan and its billing period. Cancelling and changing the card go through Stripe's customer portal.

## Considered options

- **A plan, plus credits to buy separately.** This means two products and two decisions for the user. We add credits only if paying users run out.
- **Charging for making Characters on the computer.** Anyone can remove the check from an open-source build. The charge would only work against honest users.
- **Keeping users' keys on our server.** We would hold other people's secrets. Their audio would also be converted in our Worker, which is at the Workers Free CPU limit (vibebuddy-cloud#11).
- **A free Relay, with only voice paid.** USB and the local network already give the full product. Reaching the box from another network is the clearest thing to pay for. ADR-0018 rejected a Relay that needs sign-in because 0.5.0 users had it without an account. The Relay is not released yet (2026-10-24), so that reason no longer holds.
- **Payment inside the App.** Card details would pass through our code. The website already takes payments with Stripe.
- **Cloning the user's own voice.** This needs proof of consent and protection against copying other people's voices. Deferred. Since 2026-10-28 a made Character can speak in a voice the user cloned or designed in their own ElevenLabs or Volcano Engine account, through their own key ([characters.md](../characters.md#your-own-voice)): the provider handles consent there, and we keep only the voice's id.
- **A language model writes lines from a persona.** This would bring a second kind of model into the App and break ADR-0007's rule that lines are reviewed before anyone hears them.

## Consequences

- This partly supersedes ADR-0009. The App, through the daemon, now calls a speech model to make Characters. It still never calls a language or image model.
- This partly supersedes ADR-0012 and ADR-0018. The Relay now needs an account on the Paid plan. Linking and takeover over USB or the local network still need no account.
- This replaces ADR-0018's "voice is free" with the Free daily allowance and the Paid plan's monthly allowance. The Free daily allowance becomes much smaller (about 20 commands and 20 replies). At the current allowance of 300 replies a day, one heavy user would cost about 18 dollars a month.
- The Relay protocol gets an account session from the daemon, and a refusal when the account has no plan. Box firmware does not change. When the Relay refuses, the App says why; the box only shows that the link is down.
- The costs that made ADR-0018 reject payment still apply: invoices and tax in more than one market. Stripe Tax handles part of it. Whether users in China can pay through Stripe is still open.
- `vibebuddy-api` stays on Workers Free for now (2026-10-24). Its CPU limit stays (vibebuddy-cloud#11), so our voices must avoid converting audio in the Worker where they can. Users with their own key are not affected, because the daemon converts their audio. The Stripe integration waits; the plan is recorded so a webhook can fill it in later.
