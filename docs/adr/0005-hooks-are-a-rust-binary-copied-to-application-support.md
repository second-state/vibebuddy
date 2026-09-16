---
status: accepted
---

# Hook 改为 Rust 单二进制，复制到 Application Support

Codex 与 Claude Code 的 Hook 目前是仓库里的两个 Python 脚本，用户配置写死其绝对路径。macOS 不保证有 `python3`，App 包一旦挪动路径也会失效。我们决定把两个脚本合成一个 Rust 二进制 `beacon-hook`（隐私过滤规则照搬，测试沿用），App 启动时复制到 `~/Library/Application Support/AgentBeacon/bin/`，Hook 配置指向那里。

## 被拒绝的方案

配置直接引用 App 包内的脚本或二进制：用户把 App 从下载目录拖进应用程序文件夹后 Hook 就断了，而 Hook 的失败是静默的。

## 后果

Hook 的行为变化要同时改 Rust 与文档；Python 脚本在 App 接管后退役。Codex 的 `/hooks` 信任仍只能由人完成。
