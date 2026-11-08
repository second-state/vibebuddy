# ZCode Adapter

Connect ZCode in Settings → Agents on either App, then start a new ZCode session. Repair updates the plugin's command when the hook binary moves. Remove unregisters the plugin and deletes its manifest, preserving other files and settings.

Manual setup uses `vibebuddy-hook plan install zcode`, `vibebuddy-hook install zcode`, and `vibebuddy-hook uninstall zcode`.

The adapter uses ZCode's local plugin contract inspected at v3.17.2 (`a9fe401e`). VibeBuddy registers `~/.zcode/cli/vibebuddy-plugin` in `plugins.dirs` in `~/.zcode/cli/config.json`, with the identity `vibebuddy@inline`. It does not change `hooks.enabled`. An explicitly disabled plugin subsystem must be enabled in ZCode before connecting. ZCode forces the hook runner on for plugins. Setup also refuses user hook events with an omitted enabled setting, since those could become active; set an explicit `hooks.enabled` value in ZCode first. Project configuration may independently affect plugin enablement.

The manifest uses process hooks with argv and a two-second timeout. `UserPromptSubmit`, `PostToolUse` and `PostToolUseFailure` report working; `PermissionRequest` reports needs input; `Stop` reports done, or needs input when the local reply classifier identifies a question. Recoverable tool errors do not end the activity. Session and turn IDs namespace activity identity. Cards use `ZC:` and both Apps use the generic Agent icon. K2 uses existing host navigation; exact ZCode desktop-session selection is not provided.

Only identity, normalized status, working directory, locally classified response kind and host metadata leave the hook. Prompts, replies, tool content, errors and transcript paths are discarded. Delivery uses the existing local `/v1/agent-hooks` endpoint and fails silently with short timeouts.

Setup preserves unrelated JSON settings, keeps a private config backup, and checks file snapshots before applying staged changes. The config and manifest are separate files: a concurrent config edit can leave a staged plugin manifest unregistered; Repair is safe to retry. Existing config permissions are preserved. There is a small race between the final comparison and rename.

ZCode has no SessionEnd, Interrupt or turn-failure hook in the inspected contract. Abandoned sessions can retain cards until later events or daemon cleanup. Stop hooks may request continuation, so completion classification needs validation alongside other Stop hooks. Ordinary tool failures refresh working because the Agent can recover. Cancellation and model failures without another hook cannot be detected reliably by this adapter.

Validation includes Rust build checks for the hook, daemon and Linux App; adapter privacy and installer regression tests; tracker tests; the real daemon HTTP router; and localization checks. Source inspection establishes the plugin contract, while live ZCode delivery remains unverified. Runtime integration with a model, graphical K2 focus, and clicking through either App have not been verified. The Mac App must also be built on macOS. This adapter is not a claim of compatibility with every ZCode version.
