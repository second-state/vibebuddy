# korekore: design handoff

korekore (これこれ, "here, here!") is VibeBuddy's new face: a kawaii, Japanese-cartoon desk cheerleader, with **your vibebuddy** kept beside the name for people who already own a box. It is built as a **new UI beside the current one**, behind a switch; the current UI is not touched (see the ground rule in the implementation plan). Why: [ADR-0020](../../adr/0020-the-box-and-the-apps-share-the-korekore-look.md).

> **Since [ADR-0021](../../adr/0021-the-apps-become-korekore-and-the-user-picks-the-box-screens.md):** the apps will have only the korekore look (until Phase 2 is accepted it opens with `VIBEBUDDY_UI=korekore`), and the box's screens are the user's choice, robot by default, from one firmware image that carries both: there is no `korekore` cargo feature. Where this plan says otherwise, the ADR wins.

This folder is everything an implementer needs; the two links below are the same design as live pages (private: ask Frank for access).

- Design canvas, every screen: <https://claude.ai/artifact/9NXAKq7WS5XySKZ466YNfS>
- Design system, tokens and components: <https://claude.ai/artifact/DEx5sgRS6sgfBZPoR3sRqX>

| File | What |
| --- | --- |
| `tokens.json` | Colours, type styles, spacing, radii, sticker shadows, strokes, pixel sizes. Each token has a `usage` note. |
| `pixel/kore.json` | Kore, 20 × 18, six expressions, one palette |
| `pixel/characters.json` | Seven Characters in the shared chibi style, 20 × 25 drafts |
| `pixel/icons.json` | Agent tiles and sidebar tiles, 12 × 12 (10 × 10 glyph + border) |
| `pixel/pixel-tag-font.json` | The bitmap lowercase for the "your vibebuddy" tag |
| `images/*.png` | 8× renders of all of the above (4× for the tag) |
| [`../../../tools/korekore-pixel-art.py`](../../../tools/korekore-pixel-art.py) | Source of the pixels; regenerates `pixel/` and `images/` |

Pixel data: each sprite is a list of row strings, each character a palette key, `.` transparent. Always scale by whole numbers, nearest-neighbour.

## Principles

1. **One world, box and app.** Cream ground, Ink outlines, solid sticker shadows, rounded type, pixel accents.
2. **Loud only when it matters.** Working is calm `sky` and silent; needs input is the one `sun` moment. Colour always means state.
3. **Cheer, never nag.** "Kore kore!" to call, "Yatta!" when done, "Daijoubu, daijoubu." when something fails. Never mock a failure.

## Foundations

### Colour (`tokens.json` → `color`)

| State | Box fill (on Ink) | App text on cream/paper | Box behaviour |
| --- | --- | --- | --- |
| Ready / idle | `lavender` | `muted` | Kore breathes, today's stats rotate |
| Working | `sky` | `work-text` | silent, `...` above Kore |
| Needs input | `sun` | `wait-text` | full-screen `sun` takeover, line once |
| Done | `mint` | `done-text` | full-screen `mint` for 3 s, line once |
| Failed | `peach` | `kore-pink-text` | stays on Ink, 3 px `peach` border |
| No link | `mist`, `mist-text` | `muted-2` | whole screen `box-gray-ground`, Kore asleep in gray |
| Pomodoro | `kore-pink` | — | full-screen pink, timer in `cream` |

`sun`, `mint`, `sky`, `kore-pink` are fills: ink text on them, never white, never use them as text on cream (that is what the `*-text` tokens are for; all text pairs were checked at 4.5:1). Agent colour (`agent-*`) says who, the state word says what.

On the box the screen is RGB565: convert with `((r >> 3) << 11) | ((g >> 2) << 5) | (b >> 3)` and keep the constants beside the existing `COLOR_*` in `firmware-rs/core/src/display.rs`.

### Type

- **M PLUS Rounded 1c** (SIL OFL 1.1) for everything people read in both apps: page title 28/800, card title 17/800, labels 15/800, buttons 13/800, body 14/400, small 12/400. Bundled in both apps, weights 400, 700 and 800.
- **DotGothic16** (SIL OFL 1.1) on the box, and in the apps for machine-ish bits: state tags (WATCHING, NOT FOUND), versions, ports, timestamps, key chips. Uppercase for tags.
- The **your vibebuddy** tag is neither: it is the bitmap in `pixel/pixel-tag-font.json` (pixel-bold, 9 rows). 2 px dots, solid, in title bars and the sidebar (`kore-pink-text`); 4 px dots (3 px + 1 px grain) beside a big wordmark (`kore-pink-tag`).

### Shape

- Everything has an ink outline: 2 px (`stroke-base`) for cards, buttons, toggles, inputs; 3 px (`stroke-bold`) for windows and the selected card.
- Shadows are solid ink offsets with no blur: 4 / 6 / 8 px (`sticker-sm/md/lg`).
- Radii: pill (999) for anything you press, 16 cards, 20 windows, 10 inner stages, 4 key chips and box pills.
- Dividers inside a card: 2 px `line`.

### Pixel art

- **Kore** is the default Character: the box itself, cream shell, screen for a face, the three keys as coloured buttons on its head. Expressions `normal`, `working`, `input`, `happy`, `sad`, `sleep`; only the face changes; the firmware keeps doing the motion (breathe, sway, hop, sink, Z). The robot stays selectable.
- **Characters** share one chibi body (big head, ink outline, two-pixel eyes with a highlight, blush). The drafts fix the style; production looks keep the existing look spec (48 × 64, 16 colours, four frames) and pack format.
- **Agent tiles**: original glyphs, not vendor logos (Claude Code = chat bubble, Codex = code window, GitHub Actions = gear, OpenCode = brackets, Copilot CLI = plane, Pi = π, own events = bolt). 24 px (2×) on the box and in tickets, 36 px (3×) in agent rows.

## Screens

Every measurement on the box is at 1× (320 × 240). The canvas shows them at 1.5×.

### Box

- **On duty, calm states** (ready, working): Ink ground. Left panel 96 × 240 in `box-card`: "korekore" at the top in `kore-pink`, Kore at scale 4 (80 × 72), the state word under it (14 px, state colour), "7 done today" under that. Right: up to three tickets, x 104–312, each `box-card`, radius 8, padding 6: 24 px agent tile, title 11 px `box-text` clipped to one line, "project · time" 9 px `box-text-muted`, state pill at the right (`radius-pixel`, state fill, ink 9 px). The page line ("1 / 2") sits above the tickets.
- **Ready (idle)** may use a centred layout instead: Kore at scale 6, READY in `lavender`, three stat chips (done in `mint`, asks in `sun`, busy in `sky` numerals).
- **Needs input**: the whole screen is `sun`. Kore at scale 5 top-left with "!?" above; "これこれ!" 34 px and "KORE KORE!" 16 px in ink beside it; below, the waiting ticket in Ink (title, "project · waiting 12s", ASK pill); bottom row "+1 more waiting" left and a `K2` chip with "take me there" right.
- **Done**: the whole screen is `mint` for 3 s: Kore happy at scale 5 between pixel sparkles, "YATTA!" 36 px, the finished ticket in Ink. Then back to on duty.
- **Failed**: Ink ground, 3 px `peach` border. Kore sad at scale 5, "daijoubu, daijoubu." in `peach`, the ticket with a FAIL pill, `K2` "open the run".
- **Pomodoro** (canvas board "Pomodoro on the box"): calm on Ink while the timer runs. Left, a 60-tick dial (ticks 4 × 9 px at radius 62, elapsed ticks in the phase colour, the rest `box-face`, a cream hand) around the countdown (38 px). Right column: the phase word (FOCUS `kore-pink`, BREAK `mint`, READY `lavender`) with a PAUSED chip when paused, key hints (`K0` start / pause / resume, `HOLD K0` stop / skip), TODAY squares and the tally (`2 FOCUS 50M`), then the Character at 2× beside an agent strip. **Only when a phase ends** does the screen fill: pink for focus done ("BREAK 05:00"), lavender for break done ("FOCUS 25:00"), with the Character happy and its own `focus_done` / `break_done` line in a bubble. Paused digits dim and blink. Back on duty, a `kore-pink` badge `F 18:21` / `B 04:59` sits on the top line.
- **Agents never take the screen during a Pomodoro** (docs/pomodoro.md): on needs input only the agent strip turns `sun` ("KORE KORE! ASK" + the card) and the line plays. On a lost link only the agent strip goes gray; the timer keeps its colours.
- **The Character in a Pomodoro**: who cheers and what they say follow the Character (frames: normal while focusing, eyes closed on a break, happy when a phase ends); colours and layout never change with the Character.
- **Voice reply**: Ink ground; the question in a `box-face` bubble on the right, the buddy at scale 3 with a `box-text` bubble holding the reply on the left; "answered by" with the agent tile and `K2` "close" at the bottom.
- **Voice overlay phases** (canvas board "voice, transfers, menu sub-views"), all on the cream panel: LISTENING (meter, `12 S LEFT`), THINKING (heard text dimmed, dots, a seconds counter), REPLY (heard text dimmed above; large type, falling back to small type when it does not fit; CJK always small; oldest rows scroll away; stays 15 s), and five errors for 6 s each in `kore-pink-text` with the buddy sad: NOT HEARD, UNAVAILABLE, BUSY, VOICE ERROR, NO MICROPHONE (copy from `voice_command.rs`). An agent event while the overlay is up adds a `sun` line, AN AGENT NEEDS YOU / AN AGENT FAILED.
- **The buddy in the voice overlay is the Character the box wears**: Kore, or the Character's look in the matching frame (listening normal, thinking eyes closed, reply happy, error sad). Replies are already spoken in that Character's persona, language and voice by the daemon (docs/resident-session.md); keep that, and make the screen and the app match it.
- **Transfers**: UPDATING (firmware) or NEW CHARACTER, a 20-segment bar and `42%`, or GETTING READY while flash erases, `HOLD K2` cancel; it covers the menu and voice overlays. A short ALL SET / restarting panel after a successful transfer is proposed (today the panel just vanishes).
- **Menu deeper views**: COMPUTERS (FORGET ALL n, NEW ID, BACK), FORGET ALL?, NEW IDENTITY? (what goes, what stays), SURE? (THE BOX GETS A NEW KEY, THEN RESTARTS), same panel, warn titles in `wait-text`, the final one in `kore-pink-text`.
- **Chrome** (board "chrome, idle life, pages and touch"): the old 4 px strip and "VibeBuddy" title become a 3 px state line and the korekore name in the side panel. The top line of the card column holds MUTE (left), and on the right the Pomodoro badge, the link glyph (12 × 7: plug for USB, waves for LAN, cloud for Relay; none without a link) and STG in orange on the staging Relay.
- **Idle life**: a small move every ~20 s (nap with Z, look left and right, stretch with a hop), the stats line rotating every 3 s; ALL QUIET with today's stats when there are no cards. On a lost link with cards kept, the whole screen is gray with no moves and no rotation.
- **Pages**: `LATEST ON TOP`, `PAGE 1 OF 2`, `2 MORE`, and a segmented page bar; the box turns the page every 8 s.
- **Touch**: a 2 px `sky` outline on what was tapped (card, page line, menu row, hint); petting the buddy makes it hop with a 5 × 5 pixel heart.
- **Leisure** (board "Leisure"): skits every 20–40 s (patrol, ball, read, stars counting 1–7, hide at the right edge, startle, dream reciting today's stats, rest); sleepy after 30 min (whole screen dimmed, Z); lights out 23:00–07:00 after 90 min. Characters other than Kore walk, rest and sleep without props.
- **No link**: whole screen `box-gray-ground`, Kore asleep and grayed at scale 6 with "Z z", NO LINK in `mist`, "waiting for the Mac" in `mist-text`.
- **Menu** (K1 held), STATUS, NETWORK, LISTENING: a cream card with a 3 px ink border, radius 12, inset 10 px on `box-dim`. Title 16 px with a 2 px ink rule; rows 13 px; the selected row filled `kore-pink` with "▸"; key chips (ink on cream) along a ruled bottom line. LISTENING shows Kore (input) beside a 16-bar level meter (green, then `sun`, then orange near the top) and "12 S LEFT · let go to send".
- **Boot**: Ink ground, Kore happy, "korekore" 40 px rounded, the bitmap tag in `kore-pink` at 2 px dots, the firmware version.

### Desktop app (Mac and Linux)

- **Window**: cream, 3 px ink border, radius 20, `sticker-lg`. Minimum about 900 × 760.
- **Sidebar**, 240 px, paper, 3 px ink right edge: window controls; "korekore" 26/800 with the bitmap tag under it; the **mini box** (208 × 156, 3 px ink, radius 16, `sticker-sm`), a redrawing of the box's current screen from `/v1/status` (not `/v1/device/screenshot`, which ties up the UART bridge); a status line in DotGothic16 12 ("BOX ONLINE · USB", "7 done · 3 asks today"); nav rows with a 24 px tile and a 15/800 label, the current one filled ink with cream text; the version at the bottom.
- **Order**: Character, Agents, Conversation (only while the resident session is on), Device, General, Advanced.
- **Content**: title 28/800 with a one-line subtitle right-aligned in `muted`; then cards.
- **Character**: a volume card (pink bar in an ink-bordered track, "Play on box" in `sun`); a 3-column grid of character cards (sprite on a `box-card` stage 104 px high, name, ▷ and Use, one-line summary); the card in use has a 3 px `kore-pink` border and "IN USE" in `kore-pink-text`; the last cell is the dashed "Make your own look".
- **Agents**: Kore with a speech bubble for the privacy note; one card of agent rows (36 px tile, name, pixel state tag + detail, Repair/Remove or Connect or nothing); a Terminal tabs card.
- **Conversation**: "Answered by" chips (Automatic, Codex with tile, Claude Code with tile; the chosen one ink), Refresh at the right; a paper card with your words right in ink bubbles and the buddy's replies left in cream bubbles with a 1× avatar of **the Character the box wears**; a footnote in pixel type.
- **Device**: the box screen big (352 × 264) with Refresh / Save image and the capture time; beside it a card with Link, Firmware ("· UP TO DATE" in `done-text`), "Make the box blink" (sun) and "Flash from file…"; below, Wi-Fi and Paired computers cards.
- **General**: Language (pill picker), three toggles, Updates (toggle, status line in pixel type, Check now), an About card with the small lockup, the version and "Report a problem…".
- **Advanced**: an Ink card showing the daemon (RUNNING in `mint`, version, address, uptime) with "Restart daemon" in `sun`; Logs and Diagnostics cards; the config path in a dashed box.
- **Onboarding** (same window style): Welcome (pink Kore stage, lockup, "Kore kore! I'll call you when your agents need you.", "Let's go" in `sun`, a line for existing VibeBuddy owners), Find the box (the box blinks `sun` with "HELLO!"; "Is this your box blinking?"; a dashed note when the box runs other firmware), Connect agents (checkbox rows with tiles, "Connect 3"), Pick a character (cards, write progress as a 20-segment bar), Wi-Fi (network list with pixel signal bars, password, Join / Skip). Step chips along the bottom: done `mint`, current ink.
- **Menu bar**: Kore at 1× (colour online, a `sun` dot when someone needs you, gray asleep without a link). Clicking opens a 360 px panel: lockup, Ink status strip with Kore and "WORKING · 3 AGENTS", mode chips, three stat chips, then Settings…, Check for updates…, Report a problem…, Quit.

- **Every other app state** is drawn on three canvas boards: "App · Device & firmware states" (box chooser, not found, pin notice, takeover, other firmware, unsupported board, firmware download states, write and flash progress with time left / route / Cancel, done, replug, rolled back, Wi-Fi joining and its five errors, paired computers, screenshot states), "App · Agents, Conversation, Character states" (hook write sheet with the diff, Codex trust-again row, terminal tab permissions, conversation loading / empty / error / needs Codex / needs Claude Code with the Anthropic API key field, custom look import and its errors) and "App · alerts, notifications, banners, onboarding end" (the five confirmations, four system notifications, unsupported-app banner, update states, launch at login, All set, flashing, newer firmware, starting the daemon). Where a canvas card shows `[…]`, use the exact copy already in the code.

## Implementation plan

Where things live today was mapped from the code; line numbers are as of `cc16829`.

### Ground rule: build beside, never over

The current UI (settings window, onboarding, menu bar item, box screens) is finished and shipped. The korekore UI is **new code beside it**:

- Do not edit the current views or their tests, screenshots and docs. New UI goes in new files. The only edits to existing files are the small hooks that read the switch and hand over to the korekore UI (`AppDelegate.swift`, `desktop/src/main.rs` `view()`, the firmware feature gate).
- Call the shared layers directly (`AppModel`, `DaemonClient`, `HookInstaller`, `VibeBuddyCore`, `desktop/src/client.rs`). If some logic only exists inside a current view, copy it into the korekore view rather than refactoring the current one; de-duplicate later, when korekore becomes the default.
- **A switch picks the UI.** Apps: a hidden setting (`ui: "korekore"` in the app config, plus a `VIBEBUDDY_UI=korekore` environment override for development), read once at launch; the current UI stays the default. Box: a cargo feature `korekore` in `firmware-rs/core` and `firmware-rs/device` that selects a second renderer; default builds stay byte-for-byte the same, so `tools/compare-display.sh` keeps passing.
- Every korekore screen gets its own previews and tests (new `examples/korekore_preview.rs`, new SwiftUI previews, new iced screenshots), never by changing the existing ones.
- Mac and Linux korekore UIs land together (AGENTS.md); new strings get zh-Hans entries and pass `tools/check-localization.py`.
- Making korekore the default, and deleting the old UI, is a later, separate decision.

### Fonts: as designed, on every surface

- **Both apps bundle M PLUS Rounded 1c** (weights 400, 700, 800) **and DotGothic16**, both SIL OFL 1.1, with their licence texts beside them (as `firmware-rs/core/fonts/` already does). Pin the source files by SHA-256 in a fetch script, as `tools/make-cjk-font.py` does for its sources.
  - Mac: copy the fonts into the bundle's Resources from `app/scripts/build-app.sh` and register them with `ATSApplicationFontsPath` in `app/Info.plist` (or `CTFontManagerRegisterFontsForURL` at launch). The current UI keeps the system font.
  - Linux: `iced::font::load` with `include_bytes!`, used only by the korekore views.
  - Full M PLUS Rounded 1c files are a few MB per weight because of the Japanese glyphs. Subsetting to Latin, kana and the CJK characters in the zh-Hans strings is allowed if the app size matters; glyphs outside the subset fall back to the system font.
- **The korekore box uses DotGothic16**: add a 16 px glyph table generated like `tools/make-cjk-font.py` (pinned source, OFL), covering ASCII including lowercase, the punctuation in use and the same 3,500 common CJK characters as `firmware-rs/core/fonts/cjk-12px.bin`. At 32 B a glyph that is about 115 KB of flash, compiled only into the `korekore` build. The current 5 × 7 and 12 px fonts stay for the current UI.

### Phase 1: foundations (shared by both UIs, no visible change)

- Generate per-platform tokens from `tokens.json`: a Swift `KorekoreTheme` in `app/Sources/VibeBuddy/Korekore/`, a Rust module `desktop/src/korekore/theme.rs`, and RGB565 constants in a new `firmware-rs/core/src/korekore/` module (RGB565: `((r >> 3) << 11) | ((g >> 2) << 5) | (b >> 3)`).
- Turn `pixel/*.json` into code with a small generator in `tools/`: a Swift sprite renderer (CGImage, `.interpolation(.none)`), a Rust one for iced (RGBA `image::Handle`, `FilterMethod::Nearest`), and `static` RGB565 tables for the firmware (flash, not heap).
- Add the UI switch in both apps (read at launch), defaulting to the current UI.
- Done: `tools/korekore-codegen.py` writes the theme and sprites for all three (`--check` runs in CI through `tools/test_korekore_codegen.py`); `tools/korekore-fonts.py` writes the subset fonts to `fonts/korekore/`. Both font families are Japanese, so the 1,605 Chinese characters M PLUS Rounded 1c lacks are filled in from Resource Han Rounded CN (OFL) at the same weight; DotGothic16 is not filled in, and a Chinese character it lacks falls back to the system font.

### Phase 2: the korekore apps (Mac and Linux in the same PR)

- **Mac**, new files under `app/Sources/VibeBuddy/Korekore/`: `KorekoreSettingsView` (sidebar + detail driven by a section enum; the current window is the `TabView` in `SettingsView.swift` l.8), one view per section (Character, Agents, Conversation, Device, General, Advanced), `KorekoreOnboardingView`, `KorekoreSidebar` with the mini box, and the shared controls (pill button, toggle, card, pixel state tag). Use `AppModel` and the other shared layers; do not modify the current views. `AppDelegate.swift` opens the korekore window instead of the current one only when the switch says so (settings window l.162, onboarding l.179).
- **Mac menu bar**: today `NSStatusItem` + `NSMenu` (`AppDelegate.swift` l.35, rebuilt l.124-152 from `VibeBuddyCore/MenuState.swift`). With the switch on, use Kore as the icon and a `MenuBarExtra(.window)` panel fed by the same `MenuState`.
- **Linux**, new module `desktop/src/korekore/`: a sidebar view and one function per section, mirroring the current `enum Tab` (`main.rs` l.67) and section methods, but in new code; the first-run guide likewise (current `guide()` ~l.905). With the switch on, `view()` (l.877) delegates to it. The korekore UI ignores `theme.rs`/Omarchy and uses its own palette; the current UI keeps following Omarchy.
- **Kore in the tray and menu bar**: new sprite code; leave `PixelFace.swift` and `desktop/src/face.rs` as they are for the current UI.
- Done in 2a (behind `VIBEBUDDY_UI=korekore`): the window, the sidebar with the mini box, the menu bar item and its panel, and Kore in the Linux tray. The mini box reads the Activities `/v1/status` now carries. The Mac views live in `app/Sources/KorekoreUI` and are drawn with sample data by `swift run --package-path app KorekorePreview <dir>`; the Linux ones by `cargo test -p vibebuddy-desktop korekore::snapshots -- --ignored` (into `target/korekore-preview`). Screenshots: `screenshots/phase-2a/`.
- **Linux differs** in two places: the tray keeps its StatusNotifierItem menu, since a tray host draws that menu itself and has no room for the Mac's panel (the icon is Kore, with the same `sun` dot and gray); and Omarchy draws the window's frame, so the view is the inside of the design's window, without its outline, shadow or window buttons.
- Done in 2b: the six pages, each with its own states from the "every state" boards (finding the box, firmware and character writes, Wi-Fi joining and errors, the hook write panel, TRUST AGAIN, terminal permissions, the conversation's loading, empty, error and needs-Codex or Claude states, Make your own look). The confirmations, notifications, banners and onboarding stay for 2c. On the Mac, `VIBEBUDDY_KOREKORE_SNAPSHOTS=<folder> swift run --package-path app VibeBuddy` (debug builds) draws every page with sample data; on Linux, `cargo test -p vibebuddy-desktop korekore::snapshots -- --ignored`. Screenshots: `screenshots/phase-2b/`.
- Not built, because the daemon has no way to do them: Cancel on a write, the time left and the route (OVER WI-FI) on the progress card, and the daemon's uptime on Advanced.
- Done in 2c: onboarding (the Mac's seven steps; Linux's five, since its install script connects the agents and sets up the start at login), the five confirmations as korekore sheets on the Mac (`KorekoreAlert`; Linux already shows them as inline panels), and the notifications worded as the alerts board has them. The look of a notification is the system's; only its words change. On Linux the link and daemon notifications come from the daemon, which doesn't know which UI is on, so only the firmware one changes there. Screenshots: `screenshots/phase-2c/`.

### Phase 3: the korekore box

- New renderer in `firmware-rs/core/src/korekore/`, compiled only with the `korekore` feature, implementing the screens in this document. Reuse the scene model, `Panel`/`PanelKind` data (`display.rs` l.143-195), the menu logic (`menu.rs`) and `Look` drawing (`look.rs`); do not change `display.rs`'s current drawing. The current renderer's entry point is `Display::render` (l.1260), dispatching to `draw_pet_scene` (l.1223), `draw_task_cards` (l.967, `card_rect` l.239), `draw_pomodoro_scene` (l.1077) and `draw_panel` (l.1816): the korekore renderer mirrors that structure.
- **Kore**: a built-in sprite drawn like a look (`draw_look_on_duty`, `Mark`, l.443-516) with its six expressions. The robot stays a Character. The VBCP pack format (`character_pack.rs`, `tools/character_pack.py`) does not change.
- **Agent tiles**: the current `agent_icon.rs` (`SIZE = 16`, vendor-derived pixels from `tools/make-agent-icons.py`) stays for the current UI; the korekore renderer uses the 12 × 12 tiles from `pixel/icons.json` at 2×.
- **Text**: draw with the DotGothic16 table (see Fonts); the current renderer's width maths assumes 6 px glyphs (`text_width` l.1749, `font::advance/width/fit/wrap`), so the korekore renderer needs its own metrics.
- **Touch**: give the korekore layout its own target rects (the current ones are `target_rect`/`target_at`, l.1666-1710).
- **Tests**: a new test file for the korekore renderer; the existing pinned pixels in `firmware-rs/core/tests/firmware.rs` stay as they are because the default build does not change. `tools/build-firmware.sh` gains a way to build the `korekore` image (for example `KOREKORE=1`, like `FAST_CLOCK=1`).
- Check the image size of the `korekore` build against the 4 MB OTA slot.

### Kore as a full Character

Kore is the default Character of the korekore UI, but there is no `characters/kore/` yet. Add it beside the others: `persona.md` (who Kore is and how it talks: warm, short, English with a dash of Japanese such as "Kore kore!", "Yatta!", "Daijoubu"), `voice.env` (until Kore has a designed voice, the built-in Jessica voice; one language, English), `lines.tsv` for every occasion in docs/characters.md, and its look from `pixel/kore.json` mapped to the four frames (normal → normal, eyes closed → sleep face, happy → happy, sad → sad). The daemon then gives Kore a persona and a voice for spoken replies like any other Character. Its voice and language are Frank's call; ask before choosing differently.

### Phase 4: Characters

For the korekore UI, redraw the seven Characters at 48 × 64, four frames, from the chibi drafts with the existing pipeline (`tools/draft-look.py`, `tools/make-look.py`, `tools/make-character.sh`), and build them as **new packs beside the shipped ones** (for example under `characters/korekore/<id>/`). Do not overwrite the current packs: boxes and the current UI keep using them.

### Not designed yet

Production Character looks, takeover animation timings, a dark theme for the apps, the Report a problem flow, the website.
