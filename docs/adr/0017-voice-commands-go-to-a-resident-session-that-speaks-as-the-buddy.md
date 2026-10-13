---
status: proposed
---

# Voice commands go to a resident Agent session that speaks as the buddy; the cloud only hears and speaks

The box has a microphone, and with Wi-Fi and the Relay (ADR-0012) it can sit away from the computer, where it is the user's only way to reach their Agents. We decided that a voice command is recognized in the cloud, handed to one long-running **resident session** (a Codex session the daemon keeps alive by driving the user's own `codex app-server` over stdio, with the user's own login), and answered with a short **spoken reply** the resident session writes itself, synthesized on the spot in the voice and persona of the Character the user picked. The cloud only turns speech into text and text into speech; the thinking is done by the user's own Agent. The resident session runs no commands and changes no files: it reads the Activities, answers, and starts or messages the Sessions that do the work, which keep their own permission prompts.

## Considered options

- **The box as a dictation microphone next to the keyboard.** macOS dictation already does this for free; the box only adds something when it is away from the computer.
- **A new Session per voice command.** Needs no change to how Agents are started, but away from the computer you can neither watch nor steer a session you just spawned.
- **Send each command into the session of the top task card.** The user doesn't know which session a sentence will land in, which is the reason voice replies to "needs input" were dropped too.
- **A cloud language model as the brain**, as Muse gadgets and xiaozhi do. Makes the buddy an assistant (CONTEXT.md reserves that word for Agents), puts model cost on us, and it would still have to drive the Agents on the computer through the computer.
- **Muse's cloud itself.** Its Gadget SDK terms forbid tokens in devices that are sold.
- **Claude Code first, through Channels.** Needs `--channels` at launch and an allow-listed plugin (ours would need `--dangerously-load-development-channels`), and Anthropic does not let third-party products use a claude.ai subscription login, which also rules out the Agent SDK and headless `claude -p` under the user's plan. Claude waits for written permission from Anthropic or a bring-your-own API key.
- **ACP (Agent Client Protocol).** One protocol for every Agent, but both adapters are Node programs sitting in front of the Agent's own server, and the second Agent it would unify is the one on hold.
- **Delivering into a session the user already has open, or into a desktop App.** Neither vendor gives a third party a way in: a terminal session is its own process with no inbound channel (hooks only go out), and the desktop Apps have no local API.
- **Recognition on the computer** (macOS speech recognition, whisper.cpp). Free and private, but Mac-only or a large bundled model, and the audio still has to cross the link.
- **Spoken replies in a separate "Agent voice"**, so the buddy's lines and Agents' answers are told apart. Rejected: one Character speaking in two ways is one presence; a second voice from the box would be a second one.

## Consequences

- ADR-0007 still holds for lines: they are written in advance. Spoken replies are a second way the buddy speaks, written at the moment they are spoken, and they may talk about what the Agents are doing. The persona rule narrows to lines.
- ADR-0009's App still calls no model. The recognition and synthesis service lives in `vibebuddy-cloud` beside the Relay but is not the Relay; it calls the TTS vendor of each Character's voice (ElevenLabs, Volcano Engine Doubao) with our keys, and stores neither audio nor text. During development, spoken replies are synthesized with Edge TTS; which vendor speaks to users is decided before release, within the one-voice rule above.
- Voice is offered only to signed-in users, so accounts come first (ADR-0016): free, with a daily allowance per account. Off by default.
- Delivery is per Agent and belongs to its Adapter, which becomes two-way. For Codex the daemon is an app-server client: `thread/start` once, `thread/resume` after a restart, `turn/start` per voice command; the spoken reply arrives as a call to a client-defined `speak` tool (`dynamicTools`), so no MCP server is needed. The thread runs read-only with approvals declined. The user can open it in the Codex App with `codex://threads/<id>`. Verified on 2026-10-13 with codex-cli 0.162.0: existing ChatGPT login reused, about 10 s per turn, memory kept across a process restart, file writes refused.
- app-server and `dynamicTools` are marked experimental, so the daemon pins the Codex versions it supports. OpenAI permits ChatGPT login for local and open-source apps; before commercial launch we apply for Sign in with ChatGPT.
- K0 long becomes push-to-talk; giving up a Pomodoro phase stays in the device menu, which already has it (amends ADR-0007, device menu).
- The resident session is one session for good, compacted rather than restarted, so lasting rules belong in a notes file in its working directory. It makes no task cards, announcements or stats.
- The resident session can be built and used from the command line first; the box side waits until the microphone works on the box (I2C scans found no ES7210, so it is probably on the ES8311 ADC).
