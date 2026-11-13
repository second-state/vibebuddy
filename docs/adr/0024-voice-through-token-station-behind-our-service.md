---
status: accepted
---

# Voice goes through Token Station, behind our service

Since 2026-11-03 our service serves no voice (ADR-0023). Each user must get their own Groq key for voice commands, and their own ElevenLabs or Volcano Engine key for spoken replies. Token Station (second-state/cloud_ai_gateway, run by bytefuture) can now do speech recognition, and it keeps a credit ledger for each user. On 2026-11-06 dragon decided that our voice service uses Token Station, and only Token Station. `vibebuddy-api` stays between the daemon and Token Station. This replaces the voice part of ADR-0023. Its Relay part stays.

- **There are two kinds of key, and no third.**
  - **Own keys.** The user sets them in the App: Groq for recognition, ElevenLabs and Volcano Engine for synthesis. They stay on the computer, and the daemon calls the vendor directly with them. This does not change.
  - **The Token Station key.** Each account has one. We make it, and only `vibebuddy-api` keeps it, encrypted in D1. It never goes to a computer.
  - The daemon's own-recognizer path goes away: `speech.json`, the `VIBEBUDDY_STT_*` variables and `VIBEBUDDY_STT_ENV_FILE`. The direct Workers AI recognizer (`CLOUDFLARE_ACCOUNT_ID`, `CLOUDFLARE_AI_TOKEN`) goes too. `VIBEBUDDY_TTS_BASE_URL` stays, because it selects a korekore service and holds no key.
- **Our service serves no voice of its own.** The Groq, Workers AI, ElevenLabs, Volcano Engine and MeloTTS paths in `vibebuddy-api` go away. Recognition through our service goes only to Token Station, through its Management API (Oraki), with the model `oraki/asr`. `vibebuddy-api` proxies each call. The daemon sends the audio to `POST /v1/voice/transcribe` with its account session. The Worker then calls Token Station with the Token Station key of that account.
- **The daemon picks a recognizer in this order:**
  1. The user's own Groq key.
  2. Our service, when the computer is signed in.
  3. Else, the box shows SET VOICE KEY.
- **The daemon picks a synthesizer in this order:** the user's own ElevenLabs or Volcano Engine key for the voice, if set; else our service. Token Station does not offer synthesis yet: a model key can use only chat and ASR. Until it does, our service answers each synthesis call with `403 {"reason":"own_key"}`, and the reply shows as text only.
- **We make the Token Station account when an account first uses our voice service, not at sign-in.** `vibebuddy-api` does these steps:
  1. It creates a Token Station user with the email `<account id>@users.korekore.ai`. Token Station gets no personal data.
  2. It grants the model `oraki/asr`.
  3. It grants a gift of 1 dollar of credit, once for each account, with priority 300 and no expiry.
  4. It makes the Token Station key.

  Each Admin call carries an `Idempotency-Key`. Thus a retry never gives a second gift.
- **We keep no limits of our own.** The daily allowance, the monthly allowance of characters and the global daily cap with its alert go away. An account may spend its voice credit in Token Station, and no more. The backstop is an enterprise spend limit that bytefuture sets on Token Station. When the credit is used up, Token Station answers `429` with the code `account_credit_exhausted`. Our service then answers `403 {"reason":"own_key"}`, and the box shows SET VOICE KEY.
- **The Paid plan sells the Relay and voice**, recognition and synthesis.
  - It is one monthly Stripe subscription: one Product, one monthly Price, in USD only. The user pays through Stripe Checkout and manages the plan in the Stripe Customer Portal.
  - On each `invoice.paid`, `vibebuddy-api` writes the period to `plans`. It also grants a Token Station credit of a fixed amount for that period, with priority 100. The credit is valid for the period only and does not roll over. Token Station spends it before the gift.
  - When the user cancels, or a payment fails, the plan runs to `period_end`. Then the account is on Free. There are no refunds. We take away nothing that the user made.
  - We set the price and the credit for each period when Token Station publishes its synthesis prices.
  - Sales open only when synthesis through Token Station works. Then `REQUIRE_PAID` goes on, and the App shows the Upgrade entry. We build the code before that.
- **When an account is deleted**, `vibebuddy-api` revokes its Token Station key and archives its Token Station user. (Token Station cannot delete a user.) It also cancels any Stripe subscription at once.
- **The App shows the voice credit that is left** on the account page. `GET /v1/account` reads it from Token Station.
- **Staging uses `https://oraki-test.bytefuture.ai`. Production uses `https://oraki.bytefuture.ai`.**

## Considered options

- **The daemon calls Token Station directly, with a key sent to the computer.** Anyone can take a key out of a computer. We would also need to hand keys out and issue them again.
- **One shared key for all accounts, with our own metering in D1.** We would build again the credit ledger that Token Station already has.
- **A monthly gift for each Free account.** It is a cost every month. Also, a person can get it many times with new GitHub accounts.
- **Our own ElevenLabs key for synthesis on the Paid plan, until Token Station has synthesis.** This brings back the voice service that we removed on 2026-11-03.
- **Keeping our global daily cap.** The enterprise spend limit on Token Station does this job.

## Consequences

- In `vibebuddy-api`, these go away: the Groq, Workers AI, ElevenLabs, Volcano Engine and MeloTTS paths; the daily allowance, the monthly allowance and the global daily cap with its alert. These are new: the Token Station keys in D1, the Admin calls to Token Station, the Stripe Checkout, Customer Portal and webhook, and the voice credit in `GET /v1/account`.
- In the daemon, the own-recognizer path and the direct Workers AI recognizer go away. The daemon no longer sends the voice reasons `allowance` and `global_cap`. When the credit is used up, the box shows SET VOICE KEY, as with no recognizer. Firmware does not change.
- Spoken replies and the lines of a Character that the user makes both spend voice credit when they go through our service. Speech made with an own key spends none.
- We wait on bytefuture for these Token Station features:
  - Synthesis for model keys.
  - Our `output_format` passed on to the vendor.
  - Streamed synthesis.

  For spoken replies, the box needs MP3 at 24 kHz mono and 48 kbps, streamed. To make the lines of a Character, the daemon needs 16 kHz PCM.
- Token Station sends `429` for a used-up credit. Elsewhere, `429` means a rate limit. Thus code that reads a Token Station `429` must read its code too.
- This ADR changes how docs/api.md, docs/resident-session.md, docs/protocol.md and docs/app.md describe voice. Those docs describe the code as it runs now. They change when the code changes.
