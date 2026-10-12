---
status: proposed
---

# Voice commands go to a resident Agent session that speaks as the buddy; the cloud only hears and speaks

The box has a microphone, and with Wi-Fi and the Relay (ADR-0012) it can sit away from the computer, where it is the user's only way to reach their Agents. We decided that a voice command is recognized in the cloud, handed to one long-running **resident session** (a Claude Code or Codex session the App starts and keeps alive), and answered with a short **spoken reply** the resident session writes itself, synthesized on the spot in the voice and persona of the Character the user picked. The cloud only turns speech into text and text into speech; the thinking is done by the user's own Agent. The resident session runs no commands and changes no files: it reads the Activities, answers, and starts or messages the Sessions that do the work, which keep their own permission prompts.

## Considered options

- **The box as a dictation microphone next to the keyboard.** macOS dictation already does this for free; the box only adds something when it is away from the computer.
- **A new Session per voice command.** Needs no change to how Agents are started, but away from the computer you can neither watch nor steer a session you just spawned.
- **Send each command into the session of the top task card.** The user doesn't know which session a sentence will land in, which is the reason voice replies to "needs input" were dropped too.
- **A cloud language model as the brain**, as Muse gadgets and xiaozhi do. Makes the buddy an assistant (CONTEXT.md reserves that word for Agents), puts model cost on us, and it would still have to drive the Agents on the computer through the computer.
- **Muse's cloud itself.** Its Gadget SDK terms forbid tokens in devices that are sold.
- **Recognition on the computer** (macOS speech recognition, whisper.cpp). Free and private, but Mac-only or a large bundled model, and the audio still has to cross the link.
- **Spoken replies in a separate "Agent voice"**, so the buddy's lines and Agents' answers are told apart. Rejected: one Character speaking in two ways is one presence; a second voice from the box would be a second one.

## Consequences

- ADR-0007 still holds for lines: they are written in advance. Spoken replies are a second way the buddy speaks, written at the moment they are spoken, and they may talk about what the Agents are doing. The persona rule narrows to lines.
- ADR-0009's App still calls no model. The recognition and synthesis service lives in `vibebuddy-cloud` beside the Relay but is not the Relay; it calls the TTS vendor of each Character's voice (ElevenLabs, Volcano Engine Doubao) with our keys, and stores neither audio nor text.
- Voice is offered only to signed-in users, so accounts come first; whether it is paid is decided with accounts. Off by default.
- Delivery is per Agent and belongs to its Adapter, which becomes two-way: Claude Code through Channels (research preview; our plugin must be allow-listed or loaded with a development flag), Codex through its app-server (experimental). Both need the resident session started our way. Both are prototyped before one is chosen.
- K0 long becomes push-to-talk; giving up a Pomodoro phase stays in the device menu, which already has it (amends ADR-0007, device menu).
- The resident session is one session for good, compacted rather than restarted, so lasting rules belong in a notes file in its working directory. It makes no task cards, announcements or stats.
- Nothing here is built until the ES7210 microphone chain works on the box.
