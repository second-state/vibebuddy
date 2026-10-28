# VibeBuddy API

How the daemon, the App and the box talk to `vibebuddy-api`, the service that holds accounts and voice (ADR-0017, ADR-0018). The service itself is the Worker `vibebuddy-api` in the closed-source `vibebuddy-cloud` repo; this is the part this repo implements.

It lives at `https://api.korekore.ai`; staging is `https://api-staging.korekore.ai`. Releases use production and every other build uses staging, as for the Relay. Production has no route until accounts are complete (vibebuddy-cloud#11).

## Accounts

An account belongs to a person and is signed into on a computer (ADR-0018). The daemon keeps the session; the App only opens the browser.

### Signing in

Sign-in follows OAuth for native apps (RFC 8252) with PKCE (RFC 7636): the browser proves who the person is, and the daemon receives the result on a loopback address.

1. The daemon makes a code verifier (43 to 128 characters of `A-Z a-z 0-9 - . _ ~`), its challenge `BASE64URL(SHA-256(verifier))` without padding, and a random `state`.
2. The App opens, in the system browser:

   ```text
   GET /v1/sign-in?redirect_uri=<loopback>&code_challenge=<challenge>&code_challenge_method=S256&state=<state>
   ```

   `redirect_uri` must be `http://127.0.0.1:<port>/…` or `http://[::1]:<port>/…`; the daemon uses `http://127.0.0.1:7331/v1/account/callback`. Anything else gets an error page.
3. The page offers the ways to sign in that are set up; the pending sign-in lasts 15 minutes.
   - **GitHub**: the browser goes through GitHub's authorization of the VibeBuddy OAuth App, which asks for no scopes. Only the numeric GitHub user id is kept.
   - **Email**, once mail is set up (ADR-0018): the person enters an address and then the 6-digit code mailed from `noreply@korekore.ai`. A code works for 10 minutes and allows 5 tries; an address gets at most 5 codes an hour.
4. The browser is redirected to `<redirect_uri>?code=<code>&state=<state>`. The daemon checks `state` and, within 2 minutes, exchanges the code once:

   ```text
   POST /v1/session
   {"code":"…","code_verifier":"…","redirect_uri":"<the same loopback URI>"}
   ```

   It answers `{"token":"…"}`, a session token of 43 base64url characters, or 400 `{"error":"invalid_grant"}` (wrong, expired or reused code, wrong verifier or redirect URI) or `{"error":"invalid_request"}`. A code is spent by the first try, whatever its outcome.

Signing in again with the same GitHub user, or the same address, finds the same account; a GitHub account and an email account are never merged. Each signed-in computer has its own session.

### Sessions

The daemon sends the token as `Authorization: Bearer <token>` and keeps it in `config_dir()/account` (mode 0600). A session doesn't expire. A 401 `{"error":"unauthorized"}` means it was revoked or the account was deleted: the daemon forgets it and reports signed out.

- `GET /v1/account` answers

  ```json
  {"id":"…","email":null,"github_id":60097,"created_at":"2026-10-23T08:44:49.885Z","plan":{"name":"free"},"today":{"day":"2026-10-23","commands":0},"limits":{"commands":0}}
  ```

  An account has an `email` or a `github_id`, never both. `plan` is `{"name":"free"}` or `{"name":"paid","period_end":"…","characters":12300,"characters_limit":100000}` (ADR-0020), where `characters` counts the billing period's spoken replies against the monthly allowance. `today` counts commands in the current UTC day, and `limits` is the daily allowance of the account's plan (0 on Free).
- `POST /v1/sign-out` revokes this session only: 204.
- `DELETE /v1/account` deletes the account with all its sessions and usage: 204.

## Voice

Both calls need `Authorization: Bearer <session>` and answer 401 without a valid one. Only daemons call them: the box's audio reaches the daemon over the link, however the box is linked (ADR-0018).

### The voice key

```text
POST /v1/account/voice-key
Authorization: Bearer <session>
{"device":"<the computer key>"}
```

It answers `{"key":"gw-…","url":"https://…","model":"oraki/asr"}`. The daemon recognizes voice commands with it at Token Station directly (ADR-0020): `POST {url}/v1/audio/transcriptions` with `Authorization: Bearer {key}` and a multipart body of `file` (a WAV with a filename such as `turn.wav`), `model` and an optional `language`; the answer is `{"text":"…"}`. The key is allowed only `model`. The service keeps no copy, so each call issues a new key and revokes that computer's key before it (computers on one account keep their own keys; past 5 computers, the key issued longest ago is revoked): the daemon keeps the key in `config_dir()`, mode 0600, and asks again only without one or when Token Station answers `revoked_api_key`, `invalid_api_key` or `expired_api_key`. The credit is the account's, not the key's, so a new key gives no new credit. The first call gives the account a one-time gift; on the Paid plan, reading the account adds credit for each billing period. When the credit is used up, Token Station answers `account_credit_exhausted` (402 or 429, by its build; go by `error.code`). Without a valid `device` the call gets 400; it gets 404 when the service has no Token Station, and 502 `{"reason":"unavailable"}` when Token Station fails.

### Recognizing a voice command

```text
POST /v1/voice/transcribe[?language=zh]
Content-Type: audio/wav

<one turn as 16-bit mono PCM WAV, at most 2 MB>
```

It answers `{"text":"What is the weather tomorrow?","language":"English"}`. `language` (ISO 639-1) is optional; unset, the recognizer detects it, and the answer names it as the recognizer does (`English` from Groq, `en` from Workers AI). A body that isn't a WAV, or a malformed language, gets 400; a larger body 413; when every recognizer fails, 502.

### Speaking a reply

```text
POST /v1/voice/speak
{"text":"…","voice":"elevenlabs:<voice_id>"}
```

It streams back `audio/mpeg`: MPEG-2 Layer III, 24 kHz mono, 48 kbps, the only rate the box's speaker runs at; it may begin with an ID3 tag. `text` is 1 to 1000 characters. `voice` names the vendor and its voice: `elevenlabs:<voice_id>` or `volc:<speaker>` (a Doubao 1.0 voice); the daemon maps each Character to one. A bad request gets 400; a synthesis that fails before any audio 502; one that fails midway cuts the stream short.

### Speaking a Character's line

```text
POST /v1/voice/speak
{"text":"…","voice":"elevenlabs:<voice_id>","for":"character"}
```

A line for a Character the user is making (ADR-0020). It streams back `audio/pcm;rate=16000`: 16 kHz mono 16-bit little-endian PCM, which the daemon trims, normalizes and packs as ADPCM. Only the Paid plan gets it; on Free the call gets `403 {"reason":"plan"}`. It counts its characters against the monthly allowance, as spoken replies do; past it the call gets `429 {"reason":"monthly"}` until the next period. A synthesis failure is 502: lines never come in another voice.

### The Paid plan and its allowances

Recognition and speech are for the Paid plan only (ADR-0020). On Free, every call gets

```text
403 {"reason":"plan"}
```

On the Paid plan, each recognition counts one command against the account's UTC day: 100 a day. Past it the call gets

```text
429 {"reason":"allowance"}
```

until 00:00 UTC. Each spoken reply counts its characters against the monthly allowance. The reply that crosses it still passes; after it the call gets

```text
429 {"reason":"monthly"}
```

until the next billing period. The service sets the numbers. A use is counted once the request is found valid, so a bad request gets 400 even on Free. `GET /v1/account` shows where the day and the period stand.

When voice as a whole reaches its daily spending cap, every call gets

```text
429 {"reason":"global_cap"}
```

until 00:00 UTC, whatever the account's allowance.

The `/v0/voice/…` calls take the same bodies without a session and run on staging only, until the daemon has moved to v1.
