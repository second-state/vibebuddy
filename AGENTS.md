# Vibe Buddy

See `CONTEXT.md` for domain vocabulary, `docs/architecture.md` for implementation boundaries, and `LESSONS.md` for lessons learned.

## Language conventions

The project is switching to English as it goes open source:

- Code, comments, logs, error messages, script output and commit messages are all in English.
- App UI strings use the English text as the key in source; Chinese translations go in `app/Localization/zh-Hans.lproj`. After changing UI strings, run `tools/check-localization.py`. English UI wording follows the "English UI terms" section of `CONTEXT.md`.
- Docs (`*.md`) are all in English. The one exception is `README.zh-CN.md`, the Chinese version of `README.md`: the two match section for section, so when you change one, update the other.

## Agent skills

### Issue tracker

Issues and specs both live in GitHub Issues (longzhi/vibe-buddy), managed with `gh`. See `docs/agents/issue-tracker.md`.

### Triage labels

The five triage labels keep their default names. See `docs/agents/triage-labels.md`.

### Domain docs

Single context: the root `CONTEXT.md` plus `docs/adr/`. See `docs/agents/domain.md`.
