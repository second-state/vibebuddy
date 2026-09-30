---
status: accepted
---

# Hooks become a single Rust binary, copied to Application Support

The Codex and Claude Code hooks are currently two Python scripts in the repository, and the user's configuration hard-codes their absolute paths. macOS doesn't guarantee `python3`, and the path also breaks as soon as the App bundle moves. We decided to merge the two scripts into one Rust binary, `vibebuddy-hook` (the privacy-filter rules are carried over as is, and the tests are kept), which the App copies to `~/Library/Application Support/VibeBuddy/bin/` at launch; the hook configuration points there.

## Rejected option

Have the configuration reference a script or binary inside the App bundle directly: once the user drags the App from Downloads into the Applications folder, the hooks break, and hook failures are silent.

## Consequences

Changes to hook behavior must update both the Rust code and the docs; the Python scripts are retired once the App takes over. Codex's `/hooks` trust still can only be granted by a person.
