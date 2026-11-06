# VibeBuddy API

How the daemon, the App and the box talk to `vibebuddy-api`, the service that holds accounts and voice (ADR-0017, ADR-0018). The service itself is the Worker `vibebuddy-api` in the closed-source `vibebuddy-cloud` repo; this is the part this repo implements.

It lives at `https://api.korekore.ai`; staging is `https://api-staging.korekore.ai`. Releases use production and every other build uses staging, as for the Relay. `VIBEBUDDY_TTS_BASE_URL` sets another base, or `off`.

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
  {"id":"…","email":null,"github_id":60097,"created_at":"2026-10-23T08:44:49.885Z","plan":{"name":"free"},"voice_credit_micro_usd":933100}
  ```

  An account has an `email` or a `github_id`, never both. `plan` is `{"name":"free"}` or `{"name":"paid","period_end":"…"}` (ADR-0020). `voice_credit_micro_usd` is the voice credit left in Token Station, in millionths of a US dollar (ADR-0024). It is absent before the account first uses the voice service, and when Token Station can't be read at that moment; the request doesn't fail then. The daemon passes it on in `/v1/status` as `account.voice_credit_micro_usd`, and the App shows it in dollars on the account card ("Voice credit $0.93"), cut down to the cent. With no value, the App shows no line.
- `POST /v1/sign-out` revokes this session only: 204.
- `DELETE /v1/account` deletes the account with all its sessions: 204.

## Voice

Both calls need `Authorization: Bearer <session>` and answer 401 without a valid one. Only daemons call them: the box's audio reaches the daemon over the link, however the box is linked (ADR-0018).

The service serves voice through Token Station only (ADR-0024). Each account has a Token Station key, which the service makes on the account's first voice call and keeps; it never goes to a computer. The first call also gives the account a gift of 1 dollar of voice credit. The service keeps no limits of its own: an account may spend its voice credit, and no more.

The daemon calls the service when the computer is signed in and the user has no own key for that use (docs/resident-session.md). A call that the service refuses is not tried again.

### Recognizing a voice command

```text
POST /v1/voice/transcribe
Content-Type: audio/wav

<one turn as 16-bit mono PCM WAV, at most 2 MB>
```

It answers `{"text":"What is the weather tomorrow?","language":"english"}`. The recognizer detects the language; `language` is its Whisper-style name, and it can be absent. A body that isn't a WAV gets 400; a larger body 413; a recognition that fails, 502.

When the account's voice credit is used up, the call gets

```text
403 {"reason":"own_key"}
```

and the daemon tells the box `own_key` (SET VOICE KEY). A 429 is a rate limit, with Token Station's `Retry-After` and a plain-text body: the daemon reports the turn as `unavailable`, and voice stays on.

### Speaking a reply

```text
POST /v1/voice/speak
{"text":"…","voice":"elevenlabs:<voice_id>"}
```

Token Station doesn't offer synthesis yet, so for now every call gets `403 {"reason":"own_key"}`, and the reply stays text. When it does, the call streams back `audio/mpeg`: MPEG-2 Layer III, 24 kHz mono, 48 kbps, the only rate the box's speaker runs at; it may begin with an ID3 tag. `text` is 1 to 1000 characters. `voice` names the vendor and its voice: `elevenlabs:<voice_id>` or `volc:<speaker>` (a Doubao 1.0 voice); the daemon maps each Character to one.

### Speaking a Character's line

```text
POST /v1/voice/speak
{"text":"…","voice":"elevenlabs:<voice_id>","for":"character"}
```

A line for a Character the user is making (ADR-0020). It gets `403 {"reason":"own_key"}` for now too, as all synthesis does. When synthesis is offered, it streams back `audio/pcm;rate=16000`: 16 kHz mono 16-bit little-endian PCM, which the daemon trims, normalizes and packs as ADPCM.

The `/v0/voice/…` calls take the same bodies without a session and run on staging only, until the daemon has moved to v1.
