# Contributing to Vibe Buddy

Thanks for your interest in Vibe Buddy. Bug reports, fixes, new agent integrations, voices and translations are all welcome.

## Reporting bugs and ideas

Open an issue on [GitHub Issues](https://github.com/longzhi/vibe-buddy/issues). For a bug, include:

- what you did, what you expected and what happened;
- your macOS version, the app version (Settings → General) and the box's firmware build (the menu's first line);
- which agent was involved (Codex, Claude Code, GitHub Actions) if any;
- if relevant, a diagnostics bundle from Settings → Advanced → Export diagnostics… (logs, config and build IDs, without hook payloads). It still contains session IDs and working directories, so look it over before attaching it.

Issues are triaged with five labels: `needs-triage`, `needs-info`, `ready-for-agent` (the spec is complete enough for an unattended coding agent), `ready-for-human` and `wontfix`.

## Setting up

You need a Mac with Apple silicon on macOS 14 or later, plus:

- the Rust toolchain (`rustup`) and the Xcode command-line tools; a full Xcode install isn't needed;
- [`just`](https://github.com/casey/just), which runs the everyday tasks (`just` lists them);
- for firmware work: the Xtensa toolchain (`cargo install espup espflash --locked`, then `espup install --targets esp32s3`) and, to flash, the box itself (ALIENTEK ATK-DNESP32S3-BOX V1.1);
- for voice work: `ffmpeg`, and `uv` for the edge-tts previews.

The [README](README.md#development) walks through the firmware, app and daemon in more detail.

## Before you open a pull request

Run the tests for what you touched:

| You changed | Run |
| --- | --- |
| `daemon/`, `hook/`, `protocol/`, `app/` | `just test` (Rust tests plus the app self-test; the same checks CI runs before a release) |
| `firmware-rs/` | `just test-firmware` (firmware-core tests, and a pixel-by-pixel comparison of the Rust and C firmware screens) |
| `firmware/` (C) | `just test-firmware-c` |
| UI text in the app | `tools/check-localization.py` |
| `tools/make_voice_pack.py` or the voice pack format | `python3 tools/test_make_voice_pack.py` and `tools/test-voice-pack.sh` |

If your change affects what the box does, test it on a real box when you can, and say in the pull request what you checked on the device. Some behavior, like flashing, audio and the serial link, can only be verified there.

## Conventions

- **English everywhere:** code, comments, logs, error messages, script output, commit messages and docs are written in English. The one exception is [`README.zh-CN.md`](README.zh-CN.md), which mirrors `README.md` section for section; if you change one, update the other (or say in the pull request that the Chinese side still needs updating).
- **UI text:** app strings are written in English in the source and used as the translation key. Add the Chinese translation to `app/Localization/zh-Hans.lproj/Localizable.strings`, then run `tools/check-localization.py`.
- **Vocabulary:** [`CONTEXT.md`](CONTEXT.md) defines the project's terms, such as the buddy, the box, task card, announcement voice and voice pack, and the English UI wording. Use them in code, UI and docs.
- **Decisions:** significant design decisions are recorded as ADRs in [`docs/adr/`](docs/adr/). If your change reverses or adds one, include an ADR.
- **Lessons:** hard-won debugging lessons go in [`LESSONS.md`](LESSONS.md).
- **Style:** match the surrounding code: its naming, comment density and idioms.

## Commits and pull requests

- Branch from `main` and keep one topic per pull request.
- Commit messages follow a short conventional style: `feat(app): …`, `fix(hook): …`, `docs: …`, `chore(voices): …`. The subject says what changed; the body says why.
- In the pull request, describe the change, how you tested it, and anything a reviewer should check by hand.

## Voices and other assets

Any audio or artwork you add must be yours to license, or come from a source that allows redistribution under CC BY-SA 4.0. In particular:

- don't commit audio generated with edge-tts: it isn't an official API, and whether its output may be redistributed is unclear;
- for ElevenLabs voices, use their premade voices and a paid plan (the free plan carries no commercial license);
- add new covered assets to the list in [`LICENSE-ASSETS`](LICENSE-ASSETS).

[`voices/README.md`](voices/README.md) explains how voices are generated and packed.

## License

By contributing, you agree that your contributions are licensed under the same terms as the project: code under [GPL-3.0-or-later](LICENSE), and assets under [CC BY-SA 4.0](LICENSE-ASSETS).
