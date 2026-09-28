# Vibe Buddy

领域词汇见 `CONTEXT.md`，实现边界见 `docs/architecture.md`，经验教训见 `LESSONS.md`。

## Language conventions

项目在为开源转向英文：

- 代码、注释、日志、报错信息、脚本输出、commit message 一律英文。
- App 界面文案以英文为 key 写在源码里，中文译文放 `app/Localization/zh-Hans.lproj`，改文案后跑 `tools/check-localization.py`。界面里的英文说法以 `CONTEXT.md` 的「英文界面用语」为准。
- 文档（`*.md`）：新建的文档一律用英文；已有的中文文档之后整体迁移，迁移前在其中增补的内容沿用中文，避免同一个文件中英混排。

## Agent skills

### Issue tracker

Issue 与规格都建在 GitHub Issues（longzhi/vibe-buddy），用 `gh` 操作。见 `docs/agents/issue-tracker.md`。

### Triage labels

五个 triage 标签沿用默认名。见 `docs/agents/triage-labels.md`。

### Domain docs

单一上下文：根目录 `CONTEXT.md` 加 `docs/adr/`。见 `docs/agents/domain.md`。
