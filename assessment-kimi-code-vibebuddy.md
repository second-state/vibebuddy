# Kimi Code CLI Adapter Feasibility for VibeBuddy

**Date:** 2026-11-04  
**Scope:** Read-only assessment of the local Kimi Code CLI checkout at `/mnt/dell-ext/Workspace/kimi-code` and VibeBuddy at `/mnt/dell-ext/Workspace/vibebuddy-cardputer`. No implementation or tests were performed.

## Conclusion

Kimi Code CLI support is feasible with a moderate, contained adapter change. The strongest fit is VibeBuddy's existing CLI-agent path: Kimi exposes lifecycle hooks, a JSON payload on stdin, a per-session identifier, working directory and session title. VibeBuddy already has a shared hook binary, a generic agent-hook endpoint, activity aggregation, and Connect/Repair/Remove UI flows across Mac and Linux.

The main engineering risk is config management. Kimi stores hooks as `[[hooks]]` entries inside the user's shared `~/.kimi-code/config.toml`; it is not a VibeBuddy-owned file. VibeBuddy must safely merge and remove only its own entries while preserving comments, unrelated settings, and other hook definitions. Kimi documents hooks as beta, so the config/event contract also needs version-aware validation and ongoing compatibility checks.

The primary product limitation is status fidelity: Kimi emits an explicit `PermissionRequest`, but the documented `Stop` payload does not expose the final assistant text or a definitive “waiting for an answer” reason. VibeBuddy can reliably represent work, permission waits, failures, interruption and session closure; conversational questions at turn end will likely be announced as done, as with another existing CLI adapter that lacks reply text.

## What Kimi's source establishes

The checkout defines the hook event enum in `packages/node-sdk/src/config/schema.ts:7-24`; it currently accepts `PreToolUse`, `PostToolUse`, `PostToolUseFailure`, `PermissionRequest`, `PermissionResult`, `UserPromptSubmit`, `Stop`, `StopFailure`, `Interrupt`, `SessionStart`, `SessionEnd`, `SubagentStart`, `SubagentStop`, `PreCompact`, `PostCompact`, and `Notification`.

The config schema at `packages/node-sdk/src/config/schema.ts:209-216` is strict and allows only `event`, optional `matcher`, required `command`, and optional `timeout` (1–600 seconds). This is important: VibeBuddy should add hook entries, not invent extra fields or a sidecar format.

Kimi's `docs/en/customization/hooks.md` documents that each hook receives JSON on stdin with common fields including `hook_event_name`, `session_id`, `session_title`, `client_type`, and `cwd`. Event-specific data may include prompt text, tool input/output, error details, and other sensitive content. Kimi executes a configured command, and hook failures/timeouts fail open. The same docs mark the feature Beta and explain that hook output/exit codes can affect some event types.

The Kimi implementation confirms that `PermissionRequest` is fired when an approval is requested (`packages/agent-core-v2/src/features/externalHooks/agent/agentExternalHooksService.ts:200-205`), while `UserPromptSubmit` is called for user messages (`:362-379`). Thus observation hooks can represent the main activity transitions without parsing transcripts.

## Fit with VibeBuddy's existing design

VibeBuddy's generic adapter endpoint is `POST /v1/agent-hooks`. The Rust hook adapter in `hook/src/cli_agents.rs:16-30, 139-166` already reduces different CLI event schemas to five normalized events: `working`, `needs_input`, `done`, `stopped`, and `session_end`. It forwards only agent/session/event identifiers and optional turn/cwd, plus locally detected host metadata; event content is not forwarded.

The daemon accepts only the known generic-agent names in `daemon/src/agent_hooks.rs:41-50`, then maps those five event types into the common Activity tracker (`:52-93`). Adding Kimi to the existing generic path should therefore avoid new aggregation logic or a new daemon endpoint. Kimi's session ID can be namespaced as `kimi:<session_id>` using the same generic activity identity strategy (`:96-103`).

The desktop and Mac apps already use `vibebuddy-hook status`, `plan`, and `install|uninstall <agent>` as the management boundary. Linux calls those operations from `desktop/src/agents.rs:52-87`; Mac invokes the hook binary from `app/Sources/VibeBuddy/HookInstaller.swift`. The installer target list is currently five agents in `hook/src/install.rs:175-182`; CLI-owned adapter metadata lives in `hook/src/cli_agents.rs`. Kimi would need to be added consistently to these shared lists and the Apps' display/status mappings.

This follows the architectural rule in `docs/architecture.md` that there is one Adapter per Agent, with adapters translating into the shared activity model and keeping aggregation out of the Adapter. The generic adapter endpoint already provides the intended path for CLI Agents.

## Proposed event mapping

| Kimi Code event | VibeBuddy event | Assessment |
| --- | --- | --- |
| `UserPromptSubmit` | `working` | Reliable start of a user turn. |
| `PostToolUse` | `working` | Optional refresh/heartbeat; consider whether every tool call is needed, since it can be frequent. |
| `PermissionRequest` | `needs_input` | Strong explicit signal that the Agent awaits approval. |
| `Stop` | `done` | No documented final reply text/reason in its event-specific fields, so conversational questions cannot be identified safely. |
| `StopFailure` | `stopped` | Failure terminal state; do not claim success. |
| `Interrupt` | `stopped` | User interrupted the turn; matches existing stopped semantics. |
| `SessionEnd` | `session_end` | Clear the session's activities. |
| `SessionStart` | No activity, or optional no-op | It is not evidence that work has begun; avoid creating idle task cards. |
| `PermissionResult` | Usually no direct event | A subsequent `PostToolUse`/`UserPromptSubmit` can restore working. If approval resolution can leave the run idle, validate behavior before mapping. |
| Subagent events | Initially ignore | They may not share the parent session's desired card identity; investigate only if users expect separate cards. |

Event semantics and actual Kimi trigger points should be validated against the checked-out implementation and a real CLI session before shipping. In particular, the public docs list more events than the current event enum excerpt's earlier CLI docs; use the exact schema from the installed release rather than relying on a stale page.

## Configuration and install/uninstall safety

Kimi's default config is `~/.kimi-code/config.toml` (custom home may be selected with `KIMI_CODE_HOME`, as referenced in its docs). Unlike OpenCode, Copilot and Pi, VibeBuddy cannot own a separate file: Kimi's `[[hooks]]` array coexists with user configuration. Current VibeBuddy merged-config support handles JSON for Claude/Codex in `hook/src/install.rs`; it does not currently parse TOML.

The adapter needs a TOML-aware read/modify/write path that:

- Adds only VibeBuddy's hook declarations, with a stable marker or exact command identity that supports repair and removal.
- Preserves all unrelated TOML values and user hook entries; ideally preserve comments and formatting, or clearly document any formatting normalization.
- Backs up before mutation and uses an atomic replacement, matching VibeBuddy's existing safety expectations.
- Produces a dry-run `plan` showing the concrete config path and exact additions/removals before the user confirms.
- Handles missing config, malformed TOML, Kimi not installed, a custom `KIMI_CODE_HOME`, and concurrent edits without clobbering user changes.
- Quotes the hook executable safely in Kimi's command string. Kimi runs commands through a shell, so a path containing spaces or shell metacharacters needs robust quoting; do not construct a shell command by naive interpolation.

Config discovery is a concrete design detail: the Linux systemd user service and Mac app may not inherit the same environment as an interactive shell, while `KIMI_CODE_HOME` can redirect Kimi's files. Prefer a clear rule for resolving the user's configured Kimi home, and show the resolved path in the plan. If the environment variable cannot reliably be discovered from the GUI, document that limitation or provide a path override rather than silently writing the default location.

## Privacy and hook behavior

The adapter must deserialize Kimi's stdin locally and build an allowlisted payload. Do not forward `prompt`, `tool_input`, `tool_output`, error text, subagent prompt/response, or unknown fields. For the initial implementation, the likely outgoing fields are `agent: "kimi"`, `session_id`, normalized `event`, `cwd`, `session_title` only if the daemon schema is intentionally extended to use it, and host navigation metadata. The existing generic daemon schema currently does not consume a title field, and cards derive the project from cwd; carrying a title is optional and should be a deliberate product decision.

Because Kimi's hook schema is strict, configuration entries must contain only supported fields. Because hooks can block some Kimi actions, the VibeBuddy hook must always exit successfully, emit no stdout/stderr content, and have short bounded execution so a local daemon outage cannot interfere with coding. A local HTTP timeout and best-effort delivery should mirror the current CLI adapters.

## K2 navigation and session title

Kimi's documented stdin payload provides `session_id`, `session_title`, `client_type`, and `cwd`, which is enough for activity identity and a human-readable title if the daemon is extended. It does not establish a stable `kimi://` deep link or a supported command to resume a specific session.

For Linux, VibeBuddy's hook already reports ancestor PIDs and the daemon can focus the process's host window; that likely gives useful K2 behavior for terminal-hosted Kimi. For macOS, the generic CLI adapter detects its host and can bring the terminal/editor forward. Precise session navigation inside an IDE or Kimi's own desktop surface is not established by the hook contract. Avoid claiming per-session return behavior until a stable public resume/deeplink mechanism is verified.

## Cross-platform product work

The repo instructions require Mac and Linux to stay in step for visible agents. A complete product addition therefore spans more than the Linux installer:

- Shared Rust CLI-agent enum, config path resolution, TOML planner/installer, stdin parser, privacy filter and generic event payload.
- Daemon allowlist/card prefix, activity/status accounting, plus status API's last-event timestamp for Kimi.
- Linux App agent model, status display, icon/prefix mapping, Connect/Repair/Remove flow and English/Chinese strings.
- Mac App `Agent` enum and hook config/status mappings, preview of planned TOML change, localized names, and hook health status.
- Linux install script/docs, README and setup guide; matching Mac docs and both English/Chinese README sections if the relevant section changes.
- Hook integration documentation and ADR if the adapter makes a meaningful new architectural choice; update any supported-agent lists and tests/fixtures.

The existing `HooksSeen` daemon status currently has one optional timestamp per supported agent (`daemon/src/status.rs:312-320`), and UI code switches explicitly over those fields. Kimi therefore needs a `kimi` timestamp in the status response and both Apps' status extraction, otherwise the new row cannot move from “Waiting” to connected after events arrive.

## Risks and mitigations

| Risk | Level | Mitigation |
| --- | --- | --- |
| Kimi hook feature is Beta and may change | Medium-high | Pin behavior to the supported Kimi CLI release range; validate config and payload against real versions; fail safely on unknown events. |
| TOML merge could damage user configuration | High | Use a mature TOML document editor, preserve unrelated entries, backup, atomic write, and require preview/confirmation. Test malformed and concurrent-edit cases. |
| Hook command shell quoting or failure blocks/changes Kimi behavior | Medium | Quote paths safely, keep hook observation-only, short timeout, always allow/exit 0; verify Kimi's failure semantics on supported versions. |
| Prompts/tool data leak from stdin | High | Strict allowlist serialization and tests proving sensitive fields never leave the local hook. |
| Conversational questions are shown as done | Medium | Document the limitation; only add text-based detection if Kimi exposes a stable, local final-reply field without transcript parsing. |
| Frequent `PostToolUse` events burden daemon or prolong cards | Low-medium | Measure event volume; rely on bounded local HTTP behavior and map only lifecycle events needed to keep state fresh. |
| Duplicate or lost terminal events cause stale/wrong cards | Medium | Reuse activity deduplication/expiry logic; validate event order, repeated Stop and SessionEnd behavior. |
| GUI and CLI resolve different Kimi config homes | Medium | Resolve using the user's actual environment when possible and display the target path before writing. |
| K2 can focus host but not resume exact Kimi session | Medium | Treat host focus as supported baseline; do not promise deep-link navigation without public API evidence. |

## Recommended implementation sequence

1. Verify the installed Kimi Code CLI version, config-home behavior, actual event payloads and event ordering with a small local hook probe. Record raw samples locally only; never forward them.
2. Decide product semantics: task-card label/prefix, whether to display `session_title`, and how a plain `Stop` is announced.
3. Implement and unit-test TOML plan/install/uninstall safety plus a Kimi stdin-to-normalized-event adapter in the shared Rust hook binary.
4. Add Kimi to daemon event acceptance and per-agent hook health status; confirm activity identities and session expiry.
5. Add matching Mac/Linux Agents UI and localizations, using the existing preview-before-write flow.
6. Update docs and run focused automated tests plus a real Linux and Mac smoke test with the same Kimi CLI version.

## Evidence and files inspected

Kimi Code CLI:

- `packages/node-sdk/src/config/schema.ts:7-24, 209-216` — hook events and strict config fields.
- `packages/agent-core-v2/src/features/externalHooks/agent/agentExternalHooksService.ts:200-210, 362-379` — permission and user prompt event triggers.
- `docs/en/customization/hooks.md` — stdin payload, event behavior, command execution, beta warning and failure behavior.
- `docs/en/configuration/config-files.md` — `~/.kimi-code/config.toml` and `KIMI_CODE_HOME` documentation.

VibeBuddy:

- `docs/architecture.md` — adapter boundaries and generic agent endpoint.
- `hook/src/cli_agents.rs:16-30, 32-114, 139-166` — current generic CLI adapter and config-file patterns.
- `hook/src/install.rs:15-73, 167-215` — supported targets and current merged/owned config boundary.
- `daemon/src/agent_hooks.rs:13-103` — accepted agent names, normalized lifecycle mapping and identity.
- `daemon/src/status.rs:312-320` — per-agent hook last-seen status.
- `desktop/src/agents.rs:9-87` and Mac App `app/Sources/VibeBuddy/HookInstaller.swift` — App management flow.
- `desktop/src/main.rs`, `desktop/src/korekore/pages.rs`, `app/Sources/VibeBuddy/SettingsView.swift`, `app/Sources/VibeBuddy/Korekore/Pages/AgentsPage.swift` — explicit UI status/agent mappings.
- `README.md`, `packaging/linux/install.sh`, `docs/agents/domain.md`, `docs/adr/` — supported-agent documentation, Linux install behavior and domain/architecture constraints.

## Final assessment

**Feasibility: Yes. Effort: Moderate. Confidence: High that an event adapter is technically viable; medium on config-management scope until behavior is smoke-tested against the supported Kimi release.** Kimi has the lifecycle signals and identifiers VibeBuddy needs, and the existing generic adapter path is a close architectural match. The work is bounded to config-safe TOML integration, event normalization, status/UI parity and compatibility validation. The beta hook contract and lack of a documented exact-session deep link are the main limits; neither prevents useful Kimi support, especially on Linux.
