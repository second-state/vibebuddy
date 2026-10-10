# VibeBuddy

VibeBuddy turns the state of local AI agents into the picture and sound of a physical pet. This document defines domain vocabulary only and records no implementation decisions; for implementation boundaries see [`docs/architecture.md`](docs/architecture.md).

## Language

### Roles and components

**VibeBuddy**:
The product name, in use since 2026-09-15 and written as one word since 2026-10-09; it was previously called AgentBeacon. From 2026-09-16 all internal names followed: the repo `vibe-buddy`, the daemon `vibebuddyd`, the hook `vibebuddy-hook`, the CLI `vibebuddy`, the VibeBuddy Protocol, the bundle id `com.vibebuddy.app`, and the local directory `VibeBuddy`.
_Avoid_: Vibe Buddy (two words, the spelling until 2026-10-09), AgentBeacon, beacond, beacon-hook (old names, only when talking about history), 氛围助手 ("vibe assistant"; that is a description, not a name)

**The buddy (氛围小助手)**:
The only anthropomorphic presence on the device: the one who watches the Agents and speaks up for you. It is a role, not a look; at any moment it wears exactly one Character. Before 2026-09-15 it was called 小灯灵 ("little lamp sprite"); in code it is `buddy`.
_Avoid_: 小灯灵 (old name), pet (fine as a generic word, not when referring to this role), Pet, Codex pet, assistant (on its own it means an Agent, see below)

**Character (角色)**:
The identity the buddy currently wears: its look, its voice, its persona and its lines, which always travel together. Only one Character is active at a time. The default Character is the original pixel robot, also named VibeBuddy.
_Avoid_: skin, theme, avatar (the look alone), pet

**Agent**:
A local AI assistant observed by VibeBuddy, such as Codex or Claude Code. An Agent is the thing being observed, not part of VibeBuddy.
_Avoid_: client, AI, assistant; also never means a subagent inside Claude Code

**Adapter**:
The layer that translates one Agent's events into VibeBuddy domain concepts, consisting of the local privacy-filtering script and the event mapping inside the daemon. There is one Adapter per Agent; aggregation logic does not belong to the Adapter.
_Avoid_: integration, plugin, connector

**App**:
VibeBuddy's graphical interface on the Mac: a menu bar icon plus a settings window. It handles onboarding, settings, characters and firmware, and supervises the daemon. It is not a second reminder channel; everything about Agents is said by the buddy.
_Avoid_: client, panel, console, companion app

**Firmware version (固件版本)**:
The semantic version of a firmware release, such as 0.2.2. It orders releases and decides what is offered; it is independent of the App's version. The build ID next to it on the box only identifies the exact build.
_Avoid_: build, build number (those are the build ID), firmware hash

**Update manifest (更新清单)**:
The signed list of what can be installed: the latest App per platform and every released firmware with the oldest App it needs.
_Avoid_: appcast (Sparkle's feed for the macOS App only), update server, feed

**Link (链路)**:
The connection between one computer and the box. A box has at most one link at a time, to one computer, and a computer links to at most one box. A link is carried over USB, over the local network, or through the Relay; when more than one is available, USB wins, then the local network, then the Relay. Moving the link from one way to another is not a drop; the link drops only when no way is left. When the link drops, the buddy closes its eyes and turns gray; a link failure is the only thing the App announces on its own.
_Avoid_: connection, USB, serial port, Wi-Fi (those are ways of carrying the link)

**Relay (中转)**:
VibeBuddy's online service that carries a link when the computer and the box can't reach each other directly, such as when they are on different networks. It passes the link along and decides nothing about Agents. It is never required: a box works over USB or the local network without it.
_Avoid_: cloud, server, backend (too broad: later online features are not the Relay)

**Pairing (配对)**:
The lasting permission for a computer to link to a box. Pairing is done once per computer and box and needs no account; the paired computers of a box are the only ones that may link to it or take it over.
_Avoid_: binding, registration, login (an account is a separate, later thing)

**Takeover (接管)**:
A paired computer becoming the one the box links to, ending the previous computer's link. Plugging in USB always takes over; otherwise the user takes over from the App, or another paired computer links on its own once the previous one has been gone long enough. Agent activity never moves the box between computers by itself.
_Avoid_: switch, handoff, steal

**Connection (接入)**:
The hook connection between an Agent and VibeBuddy, and its status: whether it is installed, and when the last event arrived.
_Avoid_: integration, installation, configuration

**Host (运行处)**:
Where the Agent process actually lives: its own desktop app, some other app (a terminal, an editor's integrated terminal), or no host at all (SSH, a background process). It only decides where K2 takes you back to; it plays no part in activity identity, the visible state or announcements. It is determined in the hook, the only place that can see the process environment.
_Avoid_: environment, entry point, entrypoint, surface, terminal (a terminal is just one kind of host)

### Sound

**Announcement voice (播报音色)**:
The single voice the buddy speaks with, shared by all its lines. It belongs to the Character and is never picked on its own: changing voice means changing Character.
_Avoid_: sound, speech (speech means the lines themselves), TTS

**Persona (人设)**:
A Character's personality and way of talking, written down so that its lines can be written in it. A persona never sees what the Agents are working on.
_Avoid_: system prompt (that is one way to use a persona), personality setting, style

**Line (台词)**:
One finished spoken sentence in a Character's voice. Each Character has many lines, written in advance; when an announcement fires, the device picks one of them. Lines are never written at the moment they are spoken.
_Avoid_: prompt (the firmware's old name for a clip), clip (the audio of a line), phrase, script

**Look (外观)**:
How a Character appears on screen. The default Character's look is the robot, drawn by the firmware; every other look is four still key frames (normal, eyes closed, happy, sad) carried in the Character pack, which the firmware moves and marks for each state.
_Avoid_: skin, avatar, sprite (that is the image format)

**Character pack (角色包)**:
One Character bundled into a single pack: its look (unless it uses the built-in one), the audio of its lines and its name, written to a location on the device separate from the program. Changing Character means changing the Character pack, never the firmware, and the whole pack is written at once so look and voice never disagree. When the device has no Character pack, it uses the built-in default Character.
_Avoid_: voice pack (the older pack that held only five lines), 语音包, 音色包, asset pack, skin pack

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
The box's own settings on its screen, opened by holding K1: volume, mute, giving up a Pomodoro phase, and a status view. It's stepped through with short presses and closes itself; it is an overlay, not a mode, and the mode underneath keeps running. It holds only what the box owns; everything else stays in the App.
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
| 氛围小助手 | the buddy | The character; the product name is still VibeBuddy |
| 链路 | link | The "Link" section of the settings window; when it drops, say disconnected |
| 接入 / 修复 / 移除 | Connect / Repair / Remove | The three hook actions; the settings tab is called Agents, and an agent that isn't connected shows Not set up |
| 角色 | character | Where you pick who the buddy is |
| 播报音色 | announcement voice | Part of a character, never picked on its own; on its own, just say voice |
| 内置音色 | built-in voice | Jessica (English), which ships with the firmware and speaks for the built-in default character |
| 角色包 | character pack | |
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
One complete session within an Agent process. Its identifier comes from the Agent; VibeBuddy never generates one itself.
_Avoid_: connection, session instance

**Turn**:
The unit of work from the moment the user submits an input until the assistant stops generating. Codex identifies it with `turn_id` and Claude Code with `prompt_id`; the two have aligned semantics, so Turn is a general cross-Agent concept, not one Agent's private vocabulary.
_Avoid_: round, prompt, request

**Activity**:
A piece of work VibeBuddy is currently tracking, together with its current state; the smallest unit worth its own announcement. For conversational Agents, an Activity's identity is determined by the Session and Turn together; producers with no Turn, such as training jobs or CI, supply their own identifier. An Activity disappears when it ends or expires.
_Avoid_: task, Job, Task (Task refers specifically to the on-device presentation, see below)

**Task card (任务卡)**:
How an Activity is presented on the device screen. At most 3 are shown at once, with the most recently active on top. In the VibeBuddy Protocol the field that carries them is named `tasks`, a historical name from v1; it doesn't change the fact that the Activity is the domain object.
_Avoid_: card, entry, Task item

### State and notifications

**Visible state (可见状态)**:
The single state snapshot that should currently be shown on the device, aggregated from all Activities. The visible state is persistent and deduplicable: an identical snapshot is not sent twice.
_Avoid_: current state, global state, snapshot

**Announcement (播报)**:
An edge notification that must be consumed exactly once, corresponding to one short voice line. An announcement doesn't change the visible state and isn't deduplicated; the same Turn must never trigger a second announcement. Task ends that land close together are merged on the device: within a short window only the first done is spoken and the rest only change the screen; a failure after a done is still spoken, and needs input always is, since it is the only announcement that blocks the user.
_Avoid_: notification, reminder, voice event

**Occasion (时机)**:
The situation an announcement is made in, which decides which lines it draws from. Ordinary occasions follow the edge itself: needs input, done, failed, focus done, break done. Special occasions are rarer readings of the same edge: first done of the day, a long session, a milestone and late night; the daily greeting and welcome back ride the first activity after a quiet spell. One edge speaks at most one line; when several occasions fit, the rarest one wins.
_Avoid_: trigger, event (that is the protocol message), case

**Milestone (里程碑)**:
A done that brings today's completion count to a round number: the 5th, 10th or 20th.
_Avoid_: streak, combo (they imply "without a failure in between", which is not counted)

**Late night (深夜)**:
The special occasion of the first done or needs input between 23:00 and 05:00, at most once a night. The buddy never speaks up at night on its own; it only changes what it says when something happens anyway.
_Avoid_: overtime, night mode

**Daily greeting (每日问候)**:
The line said once a local calendar day, the first time the link comes up or an Agent does something, whichever comes first, worded for the time of day. It is tied to the link and to the user's work, not to the box powering on, because the box restarts many times a day and may also stay plugged in for days.
_Avoid_: boot greeting, startup sound, hello (that is the device.hello event)

**Long session (长时间工作)**:
The special occasion of the first done after today's busy time passes another whole hour (one, two, three hours): the buddy tells the user to take a break.
_Avoid_: overtime, marathon

**Welcome back (久别重逢)**:
The line said when an Agent does something after the Agents have been quiet for hours on a day that has already had its greeting.
_Avoid_: reconnect (that is the link), resume

**Form of address (称呼)**:
What the buddy calls the user in some of its lines, picked from a fixed list in the App (boss, 老板, 哥…) or none at all. Every form is synthesized in advance, so the user's own name can't be one.
_Avoid_: nickname, username, name (the user's real name is never spoken)

**Today's stats (当日战绩)**:
The count of completions, the number of needs-input requests and the busy time, accumulated per local calendar day. It describes what happened during the day, not the current state, so it is only shown when idle and isn't deduplicated.
_Avoid_: statistics, metrics, history

**Needs input (需要确认)**:
The state of an Activity that is waiting for the user to respond. It ranks above working in the global priority, because it answers "what do I most need you to do right now".
_Avoid_: blocked, waiting, pending

**Program status (程序状态)**:
What a program says about itself to its terminal through OSC 7501: idle, working, done, blocked or error. It is a vocabulary at the edge, translated rather than adopted: blocked is Needs input (whatever its kind: permission, question or auth), working and done keep their meaning, and error, like an interrupted run's idle, ends an Activity without success.
_Avoid_: blocked, idle or error for VibeBuddy's own states
