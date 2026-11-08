# Investigation Report: Kimi Code CLI Support in VibeBuddy

**Date:** 2026-11-04
**Scope:** Whether the VibeBuddy desktop app, especially its Linux package, currently supports monitoring Kimi Code CLI sessions.

## 1. Objective

Determine whether Kimi Code CLI is a supported VibeBuddy agent, and distinguish VibeBuddy support from Kimi Code CLI's own Linux and IDE capabilities.

## 2. Methodology

Reviewed the repository README, Linux installer, and hook implementation. Searched the codebase for Kimi and supported-agent registrations. Cross-checked current official Kimi Code documentation and VibeBuddy's published setup guide.

## 3. Key Findings

### 3.1 Kimi Code CLI itself supports Linux

- **Evidence:** Official Kimi Code documentation says the install command targets “macOS / Linux” and starts with `kimi` (`https://moonshotai.github.io/kimi-code/en/`).
- **Interpretation:** Kimi Code CLI can run on a supported Linux machine independently of VibeBuddy.

### 3.2 Kimi Code CLI has hooks and ACP, but that does not make it a VibeBuddy agent

- **Evidence:** The official Kimi Code repository lists lifecycle hooks and ACP integration via `kimi acp`, with examples for Zed and JetBrains (`https://github.com/MoonshotAI/kimi-code`).
- **Interpretation:** Kimi offers integration mechanisms, but VibeBuddy would need an adapter that consumes those events. Kimi's MCP or ACP support is not automatically a VibeBuddy integration.

### 3.3 VibeBuddy's published supported-agent list excludes Kimi

- **Evidence:** VibeBuddy's setup guide lists Claude Code, Codex, OpenCode, GitHub Copilot CLI, Pi, and GitHub Actions; its Linux section specifically says the installer adds hooks to Claude Code and Codex (`https://vibekeys.dev/vibebuddy-setup.html`).
- **Evidence:** `README.md:51` requires Codex or Claude Code, and `README.md:59`, `README.md:119-123` document the supported adapters. No Kimi adapter is documented.
- **Interpretation:** Kimi Code CLI is not currently a supported VibeBuddy agent.

### 3.4 The Linux implementation confirms the exclusion

- **Evidence:** `packaging/linux/install.sh:2-4` names the installed agent hooks as Claude Code, Codex, OpenCode, GitHub Copilot CLI, and Pi.
- **Evidence:** `hook/src/install.rs:15-19` defines merged hook agents only as Claude and Codex; `hook/src/install.rs:176-182` defines the complete target list as Codex, Claude, OpenCode, Copilot, and Pi.
- **Interpretation:** Running `vibebuddy-hook install` will not discover or install a Kimi Code CLI integration.

## 4. Gaps & Uncertainties

No repository or official product documentation found an experimental, undocumented, or community Kimi adapter. Kimi's own hook API could make a future adapter feasible, but that would require implementation and testing in VibeBuddy.

## 5. Conclusions

**Answer: No—Kimi Code CLI does not currently have native VibeBuddy support, including on Linux.** Kimi Code CLI runs on Linux, but VibeBuddy will not automatically receive Kimi session events or show them on the box. VibeBuddy support would require a new Kimi adapter, likely based on Kimi lifecycle hooks or ACP.

**Confidence:** High, based on the current VibeBuddy installer/source allowlist plus current official Kimi and VibeBuddy documentation.

## Feasibility Assessment

Yes—this looks feasible, particularly for Linux. Kimi Code CLI has a lifecycle hook system that runs local commands and passes JSON on stdin, including `session_id`, `session_title`, `client_type`, and `cwd`. Its documented events include `UserPromptSubmit`, `PostToolUse`, `Stop`, `StopFailure`, `SessionStart`, `SessionEnd`, and `Interrupt`, which map well onto VibeBuddy’s existing normalized agent events. [Kimi hooks documentation](https://github.com/MoonshotAI/kimi-code/blob/main/docs/en/customization/hooks.md)

The likely design is to add Kimi as another “owned config file” adapter, similar to OpenCode, Copilot, and Pi. VibeBuddy would generate hook entries in `~/.kimi-code/config.toml`, invoke its existing hook binary, and translate Kimi’s lifecycle events into VibeBuddy’s `working`, `needs_input`, `done`, `stopped`, and `session_end` events. The same adapter model can serve the Mac and Linux apps; Linux already uses the shared hook installer. [Architecture](https://github.com/second-state/vibebuddy/blob/main/docs/architecture.md) · [Kimi config and event details](https://github.com/MoonshotAI/kimi-code/blob/main/docs/en/customization/hooks.md)

A few details need careful treatment:

- **Kimi hooks are documented as beta**, and the docs say configuration or behavior may change. That raises maintenance risk.
- **Kimi hook commands run through a shell.** The adapter must quote its executable path safely, including paths with spaces.
- **Hook input contains sensitive event-specific fields** such as prompts, tool input, and tool output. VibeBuddy should parse locally and forward only the minimal identifiers and lifecycle fields, following its existing privacy boundary.
- **Needs-input detection is the main product question.** Kimi has an explicit `PermissionRequest` event, which can signal approval waits. Its `Stop` event alone may not distinguish a final answer from a reply that expects the user to respond. The existing adapters already use differing heuristics where agent hooks don’t provide that distinction.
- **K2 navigation may be limited.** Kimi’s hook payload includes a session ID, but I found no documented stable desktop-app deep link. On Linux, VibeBuddy can likely focus the terminal process window using its existing host-process approach; opening a precise Kimi session may require additional support.

So the adapter itself appears straightforward, with event normalization and safe config editing as the core work. Before implementation, I’d confirm the current Kimi hook schema against a real CLI run and decide what VibeBuddy should report when Kimi stops without a clear “waiting for user” signal. Sources: [Kimi lifecycle hooks](https://github.com/MoonshotAI/kimi-code/blob/main/docs/en/customization/hooks.md), [VibeBuddy agent adapters](https://github.com/second-state/vibebuddy/blob/main/hook/src/cli_agents.rs), [VibeBuddy architecture](https://github.com/second-state/vibebuddy/blob/main/docs/architecture.md).
