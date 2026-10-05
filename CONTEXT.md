# Vibe Buddy

Vibe Buddy turns the state of local AI agents into the picture and sound of a physical pet. This document defines domain vocabulary only and records no implementation decisions; for implementation boundaries see [`docs/architecture.md`](docs/architecture.md).

## Language

### Roles and components

**Vibe Buddy**:
The product name, in use since 2026-09-15; it was previously called AgentBeacon. From 2026-09-16 all internal names followed: the repo `vibe-buddy`, the daemon `vibebuddyd`, the hook `vibebuddy-hook`, the CLI `vibebuddy`, the Vibe Buddy Protocol, the bundle id `com.vibebuddy.app`, and the local directory `VibeBuddy`.
_Avoid_: AgentBeacon, beacond, beacon-hook (old names, only when talking about history), 氛围助手 ("vibe assistant"; that is a description, not a name)

**The buddy (氛围小助手)**:
Vibe Buddy's original pixel character and the only anthropomorphic presence on the device; in English it is also called Vibe Buddy. Before 2026-09-15 it was called 小灯灵 ("little lamp sprite"); in code it is `buddy`.
_Avoid_: 小灯灵 (old name), pet (fine as a generic word, not when referring to this character), Pet, Codex pet, assistant (on its own it means an Agent, see below)

**Agent**:
A local AI assistant observed by Vibe Buddy, such as Codex or Claude Code. An Agent is the thing being observed, not part of Vibe Buddy.
_Avoid_: client, AI, assistant; also never means a subagent inside Claude Code

**Adapter**:
The layer that translates one Agent's events into Vibe Buddy domain concepts, consisting of the local privacy-filtering script and the event mapping inside the daemon. There is one Adapter per Agent; aggregation logic does not belong to the Adapter.
_Avoid_: integration, plugin, connector

**App**:
Vibe Buddy's graphical interface on the Mac: a menu bar icon plus a settings window. It handles onboarding, settings, voices and firmware, and supervises the daemon. It is not a second reminder channel; everything about Agents is said by the buddy.
_Avoid_: client, panel, console, companion app

**Link (链路)**:
The connection between the Mac and the device. When the link drops, the buddy closes its eyes and turns gray; a link failure is the only thing the App announces on its own.
_Avoid_: connection, USB, serial port (those are one implementation of the link)

**Connection (接入)**:
The hook connection between an Agent and Vibe Buddy, and its status: whether it is installed, and when the last event arrived.
_Avoid_: integration, installation, configuration

**Host (运行处)**:
Where the Agent process actually lives: its own desktop app, some other app (a terminal, an editor's integrated terminal), or no host at all (SSH, a background process). It only decides where K2 takes you back to; it plays no part in activity identity, the visible state or announcements. It is determined in the hook, the only place that can see the process environment.
_Avoid_: environment, entry point, entrypoint, surface, terminal (a terminal is just one kind of host)

### Sound

**Announcement voice (播报音色)**:
The single voice the buddy speaks with, shared by all five lines; only one is active at a time. Each candidate voice is archived as its own set; the user picks one in the App and writes it to the device.
_Avoid_: sound, speech (speech means the lines themselves), TTS

**Voice pack (语音包)**:
The five finished lines of one announcement voice bundled into a single pack, written to a location on the device separate from the program. When the device has no voice pack, it uses the built-in set it shipped with. Changing voices means changing the voice pack, not the firmware.
_Avoid_: 音色包 ("timbre pack"), asset pack, voice assets (those are the source files in the repo)

**Mute (静音)**:
The master switch for the device speaker, toggled only on the device: by long-pressing K2, or from the device menu. When it is on, neither announcements nor the Pomodoro chime make a sound; the screen behaves as usual. It belongs to the device alone: the Mac side doesn't record it, show it or toggle it on the device's behalf.
_Avoid_: do not disturb, DND, sound off

**Volume (音量)**:
The loudness level of the device speaker, from 20 to 100, stored on the device and kept across reboots; announcements and the Pomodoro chime share it. The App's slider is only a remote control, and it always shows the value the device reports back; the device menu changes it too. The minimum is not zero: the only way to silence the speaker is mute.
_Avoid_: loudness, volume settings (that is a place in the App's UI, not this concept)

### Modes

**Mode (模式)**:
What the buddy is doing for the user right now. Only one mode is active at a time, and each mode owns the whole screen. There are three: On duty, Pomodoro and Leisure. The mode decides who owns the screen, not the voice: the speaker doesn't care about modes.
_Avoid_: scene, page, view, state (that word belongs to Agents in On duty)

**On duty (值班)**:
The default mode: watch the Agents and call you when something comes up. Task cards, the buddy's expressions and today's stats all live here. As soon as an Agent does anything, the device returns to On duty.
_Avoid_: Agent mode, assistant mode ("assistant" is already ruled out by the term Agent), work mode (the user is working during Pomodoro too)

**Pomodoro (番茄钟)**:
The device's local focus timer: 25 minutes of focus, 5 minutes of break, each phase started by the user with a button. Its state lives only in the firmware and the Mac side plays no part; it is not an Activity and produces no task cards.
_Avoid_: timer, Timer, 番茄 ("tomato"), focus mode (it collides with the focus phase)

**Phase (阶段)**:
The segment the Pomodoro is currently in: focus or break, each with three run states: not started, running and paused. The end of a phase is an edge consumed exactly once, the same kind of thing as an announcement.
_Avoid_: session

**Device menu (设备菜单)**:
The box's own settings on its screen, opened by holding K1: volume, mute, giving up a Pomodoro phase, switching to Muse on a box shared with it, and a status view. It's stepped through with short presses and closes itself; it is an overlay, not a mode, and the mode underneath keeps running. It holds only what the box owns; everything else stays in the App.
_Avoid_: settings (that's the App's window), menu on its own (that's the App's menu bar menu)

**Leisure (休闲)**:
The mode the buddy wanders off into on its own after being idle long enough in On duty. It is driven by boredom, not switched by the user; any Agent activity, a button press or a dropped link sends it straight back to On duty.
_Avoid_: entertainment mode, screensaver, idle mode (idle is a state within On duty)

**Boredom (无聊度)**:
How long the device has been continuously idle, accumulated by state rather than by message: the Agents have nothing going on, the Pomodoro isn't running, the link is healthy, and no button has been pressed. It has three levels: standby, bored and sleepy.
_Avoid_: idle time (easily confused with the idle state in On duty)

**Skit (剧目)**:
A short animation, a few seconds to a dozen or so long, played at random in Leisure mode, such as patrolling, kicking a ball or reading a book.
_Avoid_: animation, little moves (those are the blinks and stretches while idle in On duty)

### English UI terms

Fixed wording for the App's English UI and English logs. Use these words when writing English copy; don't invent synonyms.

| Chinese | English | Notes |
|---|---|---|
| 盒子 | box | The device itself; lowercase in the UI, capitalized at the start of a sentence |
| 氛围小助手 | the buddy | The character; the product name is still Vibe Buddy |
| 链路 | link | The "Link" section of the settings window; when it drops, say disconnected |
| 接入 / 修复 / 移除 | Connect / Repair / Remove | The three hook actions; the settings tab is called Agents, and an agent that isn't connected shows Not set up |
| 播报音色 | announcement voice | Where you pick the voice; on its own, just say voice |
| 内置音色 | built-in voice | Jessica (English), which ships with the firmware |
| 语音包 | voice pack | |
| 音量 / 静音 | volume / mute | |
| 模式 | mode | |
| 值班 / 番茄钟 / 休闲 | On duty / Pomodoro / Leisure | Capitalized in the UI; duty / pomodoro / leisure in code and comments |
| 剧目 | skit | The little performances in Leisure mode |
| 设备菜单 | the box's menu | On the box's screen it's titled MENU; in the App, "the menu" means the menu bar menu |
| 当日战绩 | today's stats | Written in the menu as Today: done · asks · busy |
| 需要确认 | needs input | The voice line is Hey, I need you for a sec.; counted as asks in the stats |
| 任务卡 | task card | |
| 固件 / 刷入 | firmware / flash | |
| UART 桥 / 原生 USB | UART bridge / native USB | |

### Activity lifecycle

**Session**:
One complete session within an Agent process. Its identifier comes from the Agent; Vibe Buddy never generates one itself.
_Avoid_: connection, session instance

**Turn**:
The unit of work from the moment the user submits an input until the assistant stops generating. Codex identifies it with `turn_id` and Claude Code with `prompt_id`; the two have aligned semantics, so Turn is a general cross-Agent concept, not one Agent's private vocabulary.
_Avoid_: round, prompt, request

**Activity**:
A piece of work Vibe Buddy is currently tracking, together with its current state; the smallest unit worth its own announcement. For conversational Agents, an Activity's identity is determined by the Session and Turn together; producers with no Turn, such as training jobs or CI, supply their own identifier. An Activity disappears when it ends or expires.
_Avoid_: task, Job, Task (Task refers specifically to the on-device presentation, see below)

**Task card (任务卡)**:
How an Activity is presented on the device screen. At most 3 are shown at once, with the most recently active on top. In the Vibe Buddy Protocol the field that carries them is named `tasks`, a historical name from v1; it doesn't change the fact that the Activity is the domain object.
_Avoid_: card, entry, Task item

### State and notifications

**Visible state (可见状态)**:
The single state snapshot that should currently be shown on the device, aggregated from all Activities. The visible state is persistent and deduplicable: an identical snapshot is not sent twice.
_Avoid_: current state, global state, snapshot

**Announcement (播报)**:
An edge notification that must be consumed exactly once, corresponding to one short voice line. An announcement doesn't change the visible state and isn't deduplicated; the same Turn must never trigger a second announcement.
_Avoid_: notification, reminder, voice event

**Today's stats (当日战绩)**:
The count of completions, the number of needs-input requests and the busy time, accumulated per local calendar day. It describes what happened during the day, not the current state, so it is only shown when idle and isn't deduplicated.
_Avoid_: statistics, metrics, history

**Needs input (需要确认)**:
The state of an Activity that is waiting for the user to respond. It ranks above working in the global priority, because it answers "what do I most need you to do right now".
_Avoid_: blocked, waiting, pending
