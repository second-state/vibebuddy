# Claude Code 实时适配器

## 能力边界

Claude Code 的公开生命周期 Hook 提供了 AgentBeacon 需要的全部信息，因此适配方式与 Codex 一致：只用官方 Hook，不抓取 UI，不解析 transcript。

| Claude Code Hook | AgentBeacon 状态 |
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

两个 Adapter 写入同一个聚合器，因为设备只有一块屏幕和一只小灯灵。任务卡标题带 Agent 前缀：Codex 为 `CX:`，Claude Code 为 `CC:`。两个 Agent 常常在同一个目录下工作，没有前缀就无法区分该切回哪个窗口。

前缀只能使用固件字体支持的字符：`A-Z`、`0-9` 以及 `-.:/!?>` 等少数符号。固件按单字节渲染，非 ASCII 分隔符会被拆成两个未知字形。

标题取自 git 项目根而非工作目录：直接用工作目录会把 `repo/tools` 显示成 `TOOLS`，把 worktree 显示成分支目录名。worktree 的 `.git` 是指回主仓库的文件，因此两种情况都能还原成同一个项目名。

## 隐私边界

Hook 的原始载荷含 `prompt`、`tool_input`、`tool_response`、`transcript_path`、`session_title` 和 `last_assistant_message`。[`tools/claude-hook.py`](../tools/claude-hook.py) 在发送 HTTP 前只保留：

- `session_id`
- `prompt_id`
- `hook_event_name`
- `cwd`
- `agent_id`
- `agent_type`

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
            "command": "/usr/bin/python3 /Users/dragon/workspace/agent-beacon/tools/claude-hook.py",
            "timeout": 2
          }
        ]
      }
    ]
  }
}
```

Hook 保持同步执行，不使用 `async`。异步会让事件乱序到达，而活动模型依赖顺序：迟到的 `PostToolUse` 会让已经结束的 Turn 复活。脚本本身 0.5 秒超时且失败静默，正常情况下往返不足 10 毫秒。`SessionEnd` 的所有 Hook 共享 1.5 秒预算，0.5 秒的上限在其中是安全的。
