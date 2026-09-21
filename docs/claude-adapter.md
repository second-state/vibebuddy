# Claude Code 实时适配器

## 能力边界

Claude Code 的公开生命周期 Hook 提供了 Vibe Buddy 需要的全部信息，因此适配方式与 Codex 一致：只用官方 Hook，不抓取 UI，不解析 transcript。

| Claude Code Hook | Vibe Buddy 状态 |
| --- | --- |
| `UserPromptSubmit` | 工作中 |
| `PermissionRequest` | 需要确认 |
| `PostToolUse` | 恢复工作中 |
| `SubagentStart` | 工作中，子 agent 独立成卡 |
| `SubagentStop` | 该子 agent 完成 |
| `Stop`（回复正在等待用户回答） | 需要确认 |
| `Stop`（其他回复） | 完成 |
| `StopFailure` | 空闲，标题为 `STOPPED`，不误报成功 |
| `SessionEnd` | 空闲 |

判断助手是否在等待回答的规则与 Codex 共用 [`tools/hook_filter.py`](../tools/hook_filter.py)，理由见 [ADR-0002](adr/0002-both-adapters-share-the-waiting-heuristic.md)。权限类等待不经过该规则，`PermissionRequest` 是显式事件。

## 活动身份

Claude Code 的 `prompt_id` 与 Codex 的 `turn_id` 语义对齐，都标识一个 Turn。

需要注意的是**后台 agent 共享父会话的 `session_id` 与 `prompt_id`**：2026-09-14 的实测显示，子 agent 触发的 `SubagentStart`、`SubagentStop` 及其内部的工具事件，这两个字段都与父会话相同，只有 `agent_id` 不同。因此活动身份必须是 `session_id + prompt_id + agent_id`，否则并行的子 agent 会互相覆盖。

## 与 Codex 并存

两个 Adapter 写入同一个聚合器，因为设备只有一块屏幕和一只氛围小助手。任务卡标题带 Agent 前缀：Codex 为 `CX:`，Claude Code 为 `CC:`。两个 Agent 常常在同一个目录下工作，没有前缀就无法区分该切回哪个窗口。

前缀只能使用固件字体支持的字符：`A-Z`、`0-9` 以及 `-.:/!?>` 等少数符号。固件按单字节渲染，非 ASCII 分隔符会被拆成两个未知字形。

标题取自 git 项目根而非工作目录：直接用工作目录会把 `repo/tools` 显示成 `TOOLS`，把 worktree 显示成分支目录名。worktree 的 `.git` 是指回主仓库的文件，因此两种情况都能还原成同一个项目名。

## K2 导航

K2 的落点取决于**运行处**：Agent 进程实际待在哪里。

| 运行处 | 判据 | K2 打开 |
| --- | --- | --- |
| Claude App 的 Code 会话 | 有 `CLAUDE_CODE_HOST_SESSION_ID` | `claude://code/continue?session=<桌面会话 id>` |
| 别的应用（终端、编辑器） | `__CFBundleIdentifier` 是别人 | `open -b <那个 bundle id>` |
| 没有宿主（SSH、后台进程） | 两者都没有 | 跳过，试下一个候选 |

判定在 Hook 里做，那是唯一看得见进程环境的地方；`vibebuddyd` 只做路由。Hook 上报 `surface`、`host_bundle_id` 与 `desktop_session_id` 三个派生字段，它们来自环境变量，不含用户内容。

**桌面会话 id 由 Claude App 自己给出。** App 起的 Code 会话把它放在 `CLAUDE_CODE_HOST_SESSION_ID` 里，Hook 原样上报：

```text
CLAUDE_CODE_HOST_SESSION_ID=local_44d42f48-a5cc-43c6-b95b-26407f579d39
```

早先的做法是拿 CLI `session_id` 去 `~/Library/Application Support/Claude/claude-code-sessions/` 里按 `cliSessionId` 加 `cwd` 消歧。那条路一对多——worktree 迁移、fork、每次 `claude://resume` 导入都会多出一条桌面记录——曾经打开过一个内容停在前一天的影子会话，每按一次还把整份 5.4 MB transcript 重新导入一遍（见 `LESSONS.md`）。环境变量是 App 给的权威身份，不需要猜。索引扫描只作为旧 Hook 与旧状态文件的回退保留。

**终端会话不用 deeplink。** `claude://resume` 会把终端里的会话导入成 App 里的一份副本，人却还在终端里。落点改为把宿主应用拉到前台，bundle id 直接取自 `__CFBundleIdentifier`——LaunchServices 启动应用时注入，沿进程链继承到 Hook。

这里不认识任何具体终端：读到什么 bundle id 就打开什么。Ghostty、iTerm2、WezTerm、Terminal.app、VS Code 与 Cursor 的集成终端走的都是同一条路，换一个没见过的终端也不需要改代码。不用 `TERM_PROGRAM` 正是因为那要维护一张终端名到 bundle id 的映射表。

**Claude App 的内嵌终端面板是个例外**：它的 `__CFBundleIdentifier` 也是 Claude App，但那是终端场景。区分靠 `CLAUDE_CODE_HOST_SESSION_ID` 缺席——面板里的 CLI 没有它。此时按宿主处理，K2 把 Claude App 拉到前台，人就落在那个面板上。

**不看 tty。** Agent 执行工具命令用的是非交互子进程，即使宿主是终端也报 `not a tty`（2026-09-21 在 Ghostty 里跑 codex 实测）。Hook 同样是子进程，同样没有 tty。

2026-09-15 已用本机 Claude 1.52386.6 验证 `code/continue` 使目标会话的 `lastFocusedAt` 前移，且不产生任何导入日志。

## 隐私边界

Hook 的原始载荷含 `prompt`、`tool_input`、`tool_response`、`transcript_path`、`session_title` 和 `last_assistant_message`。[`tools/claude-hook.py`](../tools/claude-hook.py) 在发送 HTTP 前只保留：

- `session_id`
- `prompt_id`
- `hook_event_name`
- `cwd`
- `agent_id`
- `agent_type`

还有三个来自进程环境、不含用户内容的派生字段：`surface`、`host_bundle_id` 与 `desktop_session_id`（见上节）。

当且仅当主会话的 `Stop` 被本机规则判定为等待回答时，额外加入派生字段 `response_kind`。子 agent 的最后一段是写给父会话的报告，不是向用户提问，因此 `SubagentStop` 不做该判定。

它只请求 `http://127.0.0.1:7331/v1/claude-hooks`，超时 0.5 秒；daemon 未运行或载荷无效时静默退出 0。

## 安装

在 `~/.claude/settings.json`（对所有项目生效）或项目的 `.claude/settings.json` 中，为上表的八个事件配置：

```json
{
  "hooks": {
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "/usr/bin/python3 /Users/dragon/workspace/vibe-buddy/tools/claude-hook.py",
            "timeout": 2
          }
        ]
      }
    ]
  }
}
```

Hook 保持同步执行，不使用 `async`。异步会让事件乱序到达，而活动模型依赖顺序：迟到的 `PostToolUse` 会让已经结束的 Turn 复活。脚本本身 0.5 秒超时且失败静默，正常情况下往返不足 10 毫秒。`SessionEnd` 的所有 Hook 共享 1.5 秒预算，0.5 秒的上限在其中是安全的。

## vibebuddy-hook

2026-09-16 起 Hook 由 Rust 二进制 `vibebuddy-hook claude` 处理（源码在 `hook/`），它的前身 `tools/claude-hook.py` 已退役，测试用例逐条搬了过去。App 把它复制到 `~/Library/Application Support/VibeBuddy/bin/vibebuddy-hook`，配置里的命令写作 `"<那个路径>" claude`；手工配置时也用这个路径，别指向 App 包内，App 挪位置会断（ADR-0005）。
