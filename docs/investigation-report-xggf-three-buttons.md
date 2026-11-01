# Investigation Report: XGGF Three Physical Buttons

**Date:** 2026-11-01
**Scope:** Read-only investigation of the sibling `/mnt/dell-ext/Workspace/XGGF` project, concentrating on stock-firmware disassembly and corroborating documentation.

## 1. Objective

Find concrete evidence for the XGGF device's three physical buttons, including their GPIO assignments and electrical polarity.

## 2. Methodology

Searched the sibling project's documentation, source, firmware-analysis artifacts, and disassembly for button, key, GPIO, ADC, and touch references. Traced the stock firmware's GPIO initialization through the per-button reader and its caller. Compared these results with the prose GPIO maps.

The analyzed stock image is identified by the sibling project as ESP32-C3 firmware v2.3.0, built 2024-03-05, with SHA-256 `4c6e034692cd84ab69dae7e7970e8314da85c1c35d0e9a3cd7f855548c1ee576` (`../../XGGF/DEV_GUIDE.md:3-7`).

## 3. Key Findings

### 3.1 Exactly three buttons are scanned

`FUN_ram_42001b20` initializes three-byte state arrays, sets the loop bound to 3, calls `FUN_ram_42001a66` with button indices 0, 1, and 2, and stores each result in a per-button state byte.

- **Evidence:** `../../XGGF/flash_dumps/flash_dump/analysis/app0_disasm_from_fixed_elf.txt:6967-6986`

```asm
42001b36: li    a2,3
...
42001b5a: li    s3,3
42001b5c: zext.b a0,s1
42001b60: jal   0x42001a66
42001b66: sb    a0,412(a5)
```

- **Evidence:** `../../XGGF/flash_dumps/flash_dump/analysis/app0_disasm_from_fixed_elf.txt:7002-7003`

```asm
42001b98: addi  s1,s1,1
42001b9a: bne   s1,s3,0x42001b5c
```

**Interpretation:** This is direct stock-firmware evidence of three independently scanned buttons, not an inference from a product description.

### 3.2 Button-to-GPIO mapping and active-low polarity

`FUN_ram_42001a66` dispatches on its button-index argument:

| Button index | GPIO | Disassembly evidence |
|---|---:|---|
| 0 | 8 | `app0_disasm_from_fixed_elf.txt:6905,6913-6916` |
| 1 | 9 | `app0_disasm_from_fixed_elf.txt:6903-6904,6932-6935` |
| 2 | 2 | `app0_disasm_from_fixed_elf.txt:6906-6907,6947-6950` |

For example, button 0 loads GPIO8 and calls the digital-read thunk:

```asm
42001a76: beqz  a0,0x42001a88
...
42001a8e: li    a0,8
42001a90: jal   0x42018e38
```

Each branch then applies `seqz` to the pin-reading result:

- **Button 0:** `../../XGGF/flash_dumps/flash_dump/analysis/app0_disasm_from_fixed_elf.txt:6916-6918`
- **Button 1:** `../../XGGF/flash_dumps/flash_dump/analysis/app0_disasm_from_fixed_elf.txt:6935-6937`
- **Button 2:** `../../XGGF/flash_dumps/flash_dump/analysis/app0_disasm_from_fixed_elf.txt:6950-6952`

**Interpretation:** A pressed button reads logic low. All three are active-low digital inputs; they are not ADC resistance-ladder buttons.

### 3.3 Three consecutive low samples are required

Each branch samples its assigned pin three times and returns true only when the accumulated active-low count equals 3.

- **Evidence:** `../../XGGF/flash_dumps/flash_dump/analysis/app0_disasm_from_fixed_elf.txt:6913-6931` (button 0), `6932-6946` (button 1), and `6947-6961` (button 2).

```asm
42001a8c: li    s0,3          # sample count
...
42001a98: seqz  a0,a0         # low -> 1
42001a9e: add   a0,a0,a5      # accumulate
...
42001abc: addi  a0,a0,-3
42001abe: seqz  a0,a0         # true iff all three were low
```

**Interpretation:** The stock firmware has a three-sample debounce/confirmation step for every button.

### 3.4 GPIO initialization corroborates the reader

Startup code calls `pinMode` with mode 1 (Arduino `INPUT`) for GPIO8, GPIO9, and GPIO2.

- **Evidence:** `../../XGGF/flash_dumps/flash_dump/analysis/app0_disasm_from_fixed_elf.txt:6888-6898`

```asm
42001a4a: li    a1,1
42001a4c: li    a0,8
42001a4e: jal   0x42018d16
42001a52: li    a1,1
42001a54: li    a0,9
42001a56: jal   0x42018d16
...
42001a5c: li    a1,1
42001a5e: li    a0,2
42001a62: j     0x42018d16
```

**Interpretation:** Initialization and reading independently agree on the same three pins. Mode 1 alone does not establish internal pull-ups, so external pull-ups or another board-level bias remain possible.

### 3.5 The sibling's prose GPIO maps are stale

- `../../XGGF/NOTES.md:295,383,432` calls GPIO3 a button/ADC input and mentions resistance-ladder buttons.
- `../../XGGF/DEV_GUIDE.md:132,137-138,144-147` labels GPIO3 a buzzer but still labels GPIO2/8/9 as outputs.
- `../../XGGF/AUDIO_SENSOR_ANALYSIS.md:219-228` proves GPIO3 is configured as LEDC PWM buzzer output.
- The raw disassembly at `../../XGGF/flash_dumps/flash_dump/analysis/app0_disasm_from_fixed_elf.txt:5051-5067` independently confirms GPIO3 is attached to LEDC channel 4.

**Interpretation:** Do not use the sibling prose maps as the button authority. The raw button-reader disassembly supersedes them.

### 3.6 Physical buttons are also confirmed operationally

The sibling's live-device control notes state that system, schedule, and alarm fields are set using physical device buttons rather than the network API.

- **Evidence:** `../../XGGF/xggf_reader/DEVICE_CONTROL.md:41-48` and `127-137`.

This corroborates the existence and practical use of physical controls, but does not establish their pin mapping.

## 4. Architecture Overview

```text
xTask_Proj (0x4200c524)
  -> button processing (0x42001dc0)
       -> three-button scanner (0x42001b20)
            -> indexed reader (0x42001a66)
                 -> index 0: digitalRead(GPIO8), active-low
                 -> index 1: digitalRead(GPIO9), active-low
                 -> index 2: digitalRead(GPIO2), active-low
```

The scanner is called from `FUN_ram_42001dc0` at `app0_disasm_from_fixed_elf.txt:7185`, and that function is called from the main project task at `app0_disasm_from_fixed_elf.txt:21453`.

## 5. Gaps & Uncertainties

| # | Gap | Impact | Follow-up |
|---|-----|--------|-----------|
| 1 | The disassembly does not provide physical positions or case labels for indices 0/1/2. | GPIO mappings are known, but “left/middle/right” or “up/down/select” cannot be assigned from this evidence alone. | Press each physical button while logging GPIO2/8/9 levels. |
| 2 | Mode 1 is `INPUT`, not `INPUT_PULLUP`. | Internal pull-up use must not be assumed. | Inspect the PCB or measure the unpressed pin voltage/bias. |
| 3 | This investigation did not decode every short-press, long-press, and menu action. | Button behavior beyond scanning and debounce is outside the established result. | Trace the per-button event flags emitted by `0x42001b20` through `0x42001dc0`. |

## 6. Conclusions

High-confidence result: the stock XGGF firmware has exactly three independently scanned, active-low digital buttons:

- **Button 0:** GPIO8
- **Button 1:** GPIO9
- **Button 2:** GPIO2

Each requires three consecutive low samples in the low-level reader. The sibling project's claims about GPIO3 ADC/resistance-ladder buttons are stale and contradicted by both the button reader and the buzzer initialization. Physical button names and positions remain unverified.
