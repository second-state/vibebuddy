# Upgrade the XGGF 64×8 status font: 3×5 hand-drawn glyphs → petabyt 5×7

## Goal and shape

Replace the 31-glyph 3×5 table in `firmware-xggf/main/status.c:110-121` with a **generated, committed 5-wide × 7-tall table covering all 95 printable ASCII characters**, derived from the vendored MIT-licensed `fonts/petabyt-5x7/font.h`. The renderer (`text()` at status.c:126-137) changes its geometry: advance 4→6 px, 3→5 columns, 5→7 rows. Every existing label fits: the longest (`MUTE OFF`, `-12C 58%` for negative temps) is exactly 8 characters, ending at x=59 of 64. No app/daemon change, no localization change (device-internal rendering only — state that in the PR per AGENTS.md).

Decisions already made: full printable ASCII (not a label subset); a minimal host PBM preview tool; generated table committed to the repo, produced by a Python generator (precedent: `tools/make-cjk-font.py` + committed `cjk-12px.bin`, `tools/korekore-fonts.py` provenance style).

## Step 1 — Generator `tools/make-xggf-font.py`

- Parses the vendored `fonts/petabyt-5x7/font.h` (7 rows × 5 cols of `" #"` ASCII art) and emits `firmware-xggf/main/font.h` + `firmware-xggf/main/font.c`.
- Table format mirrors the existing style: `typedef struct { char c; uint8_t rows[7]; } xggf_glyph;`, `extern const xggf_glyph xggf_font[XGGF_FONT_COUNT];` plus a shared lookup helper `const xggf_glyph *xggf_font_glyph(char c)` (NULL on miss). Bits: bit 4 = leftmost column, so rendering tests `rows[y] & (0x10u >> col)`. All `const` → rodata/flash, zero RAM.
- Generator rules for upstream quirks: skip the `{0, ...}` terminator, skip non-ASCII entries (`{1, ...}` degree-ish glyph), **first definition wins** on duplicates (upstream defines `-` twice; the first is the nicer row-3 3-px dash), and hard-fail unless exactly the 95 printable ASCII characters are emitted.
- Generated files carry a header: produced by `tools/make-xggf-font.py` from petabyt's 7×5 font (MIT, © 2020 Petabyte, license at `fonts/petabyt-5x7/MIT-petabyt-font.txt`) — do not edit by hand.
- Output must be plain C11, `-Wall -Wextra -Werror`-clean, no ESP-IDF includes, so it builds with the README's plain-`cc` host lines.
- New `tools/test_make_xggf_font.py` (picked up by CI's `python3 -m unittest discover -s tools`): parser spot-checks ('A' top row, 'g' descender confined to rows 5–6 inside the cell), duplicate-`-` and `{1,...}` handling, 95/95 completeness, and a **staleness check** — regenerating to a temp file must byte-match the committed `font.c`.

## Step 2 — Renderer swap in `firmware-xggf/main/status.c`

- Delete `glyphs[]`; `#include "font.h"`; use `xggf_font_glyph()`.
- `text()` changes only: start x stays 13; guard `x + 2 <` → `x + 4 < PANEL_WIDTH`; loops `y < 5`→`y < 7`, `col < 3`→`col < 5`; bit test `4u >> col` → `0x10u >> col`; row offset `y + 1` → `y` (glyphs occupy rows 0–6, row 7 stays blank — keeps lowercase descenders off the bottom edge; flip to offset 1 if the preview reads better). Advance `x += 4` → `x += 6`.
- Keep existing semantics: unknown characters advance without drawing; long strings clip safely (8 chars max — add a one-line comment stating the budget so future label authors see it).
- Linear lookup stays (≤ 8 chars × 95 entries per frame is negligible next to the 15.86 ms panel transfer). REC blink dot at `pixel(f,12,1)` is unaffected.

## Step 3 — Build wiring

`firmware-xggf/main/CMakeLists.txt:1`: add `"font.c"` to SRCS.

## Step 4 — Preview tool `firmware-xggf/tests/preview.c`

Host `cc` program linking `panel.c status.c font.c` that renders every label scenario — OFFLINE (gray), READY/WORKING/INPUT/DONE/FAILED, MUTE ON/OFF, `K0 DOWN`/`K0 LONG`, REC, `12:34`, `-12C 58%`, NO RTC, NO ENV, and the unlinked flat-face — writing each as a 1× and 8×-scaled PBM to an output directory. This finally satisfies PLAN_XGGF_64X8.md:79 ("preview every state … at native pixel resolution and enlarged") in minimal form and is the gate for glyph legibility (especially petabyt's 1-px-wide `I`). If `I` reads badly: apply one documented generator tweak (center the bar in its cell), regenerate, and note the modification in the script docstring — MIT permits it and matches the korekore subsetting precedent.

## Step 5 — Tests

- New `firmware-xggf/tests/font_test.c`: (a) all 95 printable ASCII resolve via `xggf_font_glyph`; (b) each real label string renders non-blank pixels; (c) worst-case `MUTE OFF` leaves columns ≥ 60 and row 7 untouched in the text area and never draws left of x=13; (d) two renders of the same state are byte-identical.
- `status_test.c` and `panel_test.c` must pass unchanged (they compare whole frames via memcmp, so the swap is invisible to them by design).
- `firmware-xggf/README.md` host-test section: add the `font_test.c` and `preview.c` `cc` lines alongside the existing three.

## Step 6 — Licensing paperwork

`LICENSE-ASSETS`: extend the "Not covered, because it is a third-party font under its own license" block (currently the cjk-12px.bin bullet) with `fonts/petabyt-5x7/font.h` and the derived `firmware-xggf/main/font.c` — petabyt 7×5 font by Daniel C (Petabyte), MIT, license file kept beside the source. MIT is GPLv3-compatible, so linking into the firmware is sound. Vendored upstream files stay byte-verbatim.

## Step 7 — Docs

- `firmware-xggf/README.md`: font provenance note + new test commands.
- `PLAN_XGGF_BUTTON_FRONTEND.md:135`: reword "Extend the 3×5 glyph table only for characters actually required" to reference the generated full-ASCII 5×7 table (regeneration only needed for non-ASCII).
- `PLAN_XGGF_64X8.md:77` and the current-status section: note the 5×7 font and its provenance.
- `firmware-xggf/WORK_NOTES.md`: dated entry with table size, host-test results, preview observations and the on-device check.

## Step 8 — Verification sequence

1. `python3 -m unittest discover -s tools -p "test_*.py"` (generator + staleness tests, CI parity).
2. Host `cc` tests: existing panel/status/buttons lines unchanged, plus new font test; run `preview` and inspect the PBMs (visual gate for offset/`I`).
3. `tools/check-localization.py` (expected unaffected; cheap).
4. ESP-IDF build with `IDF_TOOLS_PATH=/home/dabajabaza/.espressif-vibe` (WORK_NOTES.md:37): confirm binary grows only ~0.6 KB rodata.
5. Flash under `flock ../DEVICE.lock` and drive all states via `firmware-xggf/tests/physical_serial.py`; confirm every label is legible at normal viewing distance, the `panel_us` diagnostic stays ~15.86 ms and no flicker appeared.

## Out of scope

No lowercase/mixed-case labels, scrolling text, button-menu frontend, or app-side changes; no modification of the vendored `fonts/petabyt-5x7/` files beyond what Step 6 documents.

## Risks

- Upstream quirks are mechanically handled (duplicate `-`, `{1,...}`, terminator); the 1-px `I` is the one visual risk — preview-gated with a documented fallback fix.
- 8-character label ceiling at 6 px advance — clipping is pre-existing safe behavior, called out in a comment and in tests.
- Flash cost +~580 B rodata, RAM unchanged.