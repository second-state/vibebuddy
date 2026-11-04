# XGGF RTC evidence and validation

The stock binary's reader at `0x4200161E` reads seven bytes starting at register
zero; its writer at `0x420018BE` writes the same BCD layout. The recovered
`/api/time` handler begins at `0x4200FC86`. Stock startup reads two control bytes
at `0xE0`, sets bit 5 in the first and clears bit 4 in the second after power-on.
These addresses were checked in `app0_disasm_from_fixed_elf.txt` from the existing
unit backup analysis outside this repository.

This is consistent with an **RX-8025-compatible protocol**, an inference from
stock transactions rather than a confirmed package marking. The
[Epson RX-8025SA/NB manual](https://download.epsondevice.com/td/pdf/app/RX-8025SA_en.pdf)
describes register E bit 5 as 24-hour mode and register F bits 4/5 as power-on
reset and oscillator validity. Register addresses occupy the high nibble on the
wire. The
[RX8010SJ manual](https://download.epsondevice.com/td/pdf/app/RX8010SJ_en.pdf)
uses a different calendar register map; the original RX8010 guess is unsupported.

`rtc.c` validates the complete BCD calendar, 24-hour mode, oscillator status and
power-on flag. An invalid RTC produces NO TIME until a valid desktop heartbeat.
The experimental writer preserves other control bits, selects 24-hour mode,
clears power/voltage flags, arms oscillator-stop detection, writes the timestamp
and checks readback. It retries failed writes on later heartbeats.

`XGGF_RTC_WRITE` defaults OFF until hardware retention is accepted. A stale NVS
clock timestamp cannot measure powered-off time and is never used to invent the
time. The daily Pomodoro tally is persisted independently.

Required final check: sync with writes enabled, stop the daemon, remove all
external power while retaining the RTC backup supply, reconnect and check that
the clock is correct before any daemon heartbeat. A reset alone does not prove
power-off retention. Exact package identity requires a readable chip marking.
