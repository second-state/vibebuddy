# VibeBuddy

See `CONTEXT.md` for domain vocabulary, `docs/architecture.md` for implementation boundaries, and `LESSONS.md` for lessons learned.

## Language conventions

The project is switching to English as it goes open source:

- Code, comments, logs, error messages, script output and commit messages are all in English.
- App UI strings use the English text as the key in source; Chinese translations go in `app/Localization/zh-Hans.lproj`. After changing UI strings, run `tools/check-localization.py`. English UI wording follows the "English UI terms" section of `CONTEXT.md`.
- Docs (`*.md`) are all in English. The one exception is `README.zh-CN.md`, the Chinese version of `README.md`: the two match section for section, so when you change one, update the other.

## Mac and Linux apps stay in step

The Mac app (`app/`) and the Linux app (`desktop/`) are one product on two platforms. When a change adds, removes or changes something the user sees or does in one of them (a setting, a tab, a menu item, a Character, a supported agent, an update or K2 behaviour), make the matching change in the other in the same PR. If the other platform can't do it, say so in the PR and in the docs, with what it does instead. UI strings get their zh-Hans translations on both sides.

## Agent skills

### Issue tracker

Issues and specs both live in GitHub Issues (second-state/vibebuddy), managed with `gh`. See `docs/agents/issue-tracker.md`.

### Triage labels

The five triage labels keep their default names. See `docs/agents/triage-labels.md`.

### Domain docs

Single context: the root `CONTEXT.md` plus `docs/adr/`. See `docs/agents/domain.md`.
