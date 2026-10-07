---
status: accepted
---

# Firmware and App release separately, announced by a signed update manifest on our own domain

Until now the firmware shipped only inside the App: one `vX.Y.Z` tag built both, the Device tab compared the box's build hash with the bundled one, and nothing ever checked the internet for updates (`docs/app.md`, Firmware updates). We now want to release firmware fixes without an App release, and we want installed Apps to notice new versions of either so users get onto them quickly. That needs three things the old model deliberately avoided: a firmware version that orders releases, a compatibility rule between the two, and an update source the daemon polls.

## Decision

**Versions and releases**

- Firmware has its own semantic version, the `version` of `firmware-rs/device/Cargo.toml`, released with a `firmware-vX.Y.Z` tag. App tags stay `vX.Y.Z`. The firmware reports its version to the Mac alongside the existing build ID, which stays for display and for telling dirty or unreleased builds apart.
- **The App no longer bundles firmware.** Firmware is downloaded from the release it was published in. Developers flash local builds with `just flash` or "Flash from file…"; there is no special path for them in the App.

**The update manifest**

- One update manifest lists the latest App per platform, **every released firmware** with its own `min_app`, and a `min_supported_app`. Each download carries a URL and sha256; release notes are given per language (`en`, `zh-Hans`), falling back to English. CI regenerates it from the latest releases of both kinds whenever either is released.
- The manifest is signed with Ed25519 and the public key is compiled into the daemon. The box has no secure boot, so this check is the only thing between a compromised host and every user's box. A second, separate key signs App downloads for Sparkle (below). Both private keys are backed up in `~/.vibebuddy-signing`, stored as repository secrets, and set up by `tools/setup-release-signing.sh`.
- The manifest and the Sparkle appcast live in Cloudflare R2 behind a domain of our own (not chosen yet). That URL is baked into every shipped App; the download URLs inside it are not, so **downloads point at GitHub Releases for now** and can move later by regenerating the manifest. If users in China can't download reliably, the answer is a domestic CDN, which R2 would not fix. A Worker can later sit behind the same URL for staged rollout.

**Checking and offering**

- The daemon checks, so macOS and Linux share one implementation: at start, once a day, and on demand from a "Check for updates" button on the General tab. It sends platform, App version and firmware version, nothing else (no install ID). Checking is on by default in release builds, off in builds from source, and can be turned off in Settings.
- The firmware offered is the highest version whose `min_app` the running App satisfies, and only if it is newer than the box's. An App never offers a downgrade; a box with a dirty or unversioned build always gets an offer. Downloaded firmware is kept in the daemon's data directory, so reflashing and recovering a box work offline afterwards. When a download fails, the App links to the release page so the user can fetch the zip and use "Flash from file…".
- **Every update needs the user's confirmation, App and firmware alike.** macOS uses Sparkle's standard dialog (install, remind me later, skip this version). Linux shows that a new version exists, with a link and the command to rerun `install.sh`. Firmware gets a notification while the box is connected and an Update button on the Device tab; flashing never starts on its own.
- Below `min_supported_app` the App shows a banner that can't be dismissed, and Sparkle won't let the user skip; nothing stops working.

## Considered options

- **Keep firmware inside the App release.** No server needed, but every firmware fix costs an App release, and users still never hear about it.
- **Keep bundling the latest firmware release in the App** for offline onboarding. Rejected for simplicity: one source of firmware instead of two, at the cost of needing a network the first time a box is flashed.
- **List only the latest firmware.** An App older than that firmware's `min_app` would then have nothing to flash, even onto a factory box.
- **Read GitHub Releases directly.** The update URL would be GitHub's forever, and unauthenticated API calls are rate-limited per IP.
- **A dynamic server from day one.** Staged rollout is the only feature that needs one, and it can come later at the same URL.
- **Install updates silently, or flash firmware while the box is idle.** A failed flash leaves a dark box that only K0 and a replug recover, which is the wrong surprise to find in the morning.

## Consequences

- Reverses "no update checks and no downloading firmware from the internet" and the hash-only comparison in `docs/app.md`; that doc changes as each piece lands.
- Flashing a factory box during onboarding now needs the network once.
- The App contacts a Vibe Buddy service. The README says what is sent and how to turn it off.
- Breaking changes between the App and the firmware (partition layout, Character pack format, event meaning) must raise the firmware's `min_app`. Additive protocol changes don't, because receivers ignore unknown fields and events.
- If the box gets its own Wi-Fi link (#3), it can read the same manifest.
