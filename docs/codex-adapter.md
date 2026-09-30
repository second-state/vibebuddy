# Codex 实时适配器

## 能力边界

Codex 内置宠物的素材、动画状态机和任务卡片渲染没有公开为宠物 API，不能可靠地逐帧镜像到外部设备。可依赖的公开接口是 Codex 生命周期 Hook。

Vibe Buddy 使用以下映射：

| Codex Hook | Vibe Buddy 状态 |
| --- | --- |
| `UserPromptSubmit` | 工作中 |
| `PermissionRequest` | 需要确认 |
| `PostToolUse` | 恢复工作中 |
| `Stop`（回复正在等待用户回答） | 需要确认 |
| `Stop`（其他回复） | 完成 |
| `Interrupt` | 空闲，标题为 `INTERRUPTED`，不误报成功 |
| `SessionEnd` | 空闲 |

多会话由 `vibebuddyd` 聚合：任务卡按最近活动排序，最多 3 张；需要确认的会话优先控制宠物表情。活动身份由 `session_id` 与 `turn_id` 共同确定。每个已跟踪、且不等待用户回答的 turn，其 `Stop` 都会产生一次完成通知，即使画面仍需显示其他工作中的任务；同一 turn 的重复 `Stop` 不会重复播报。等待回答的 turn 保持为需要确认，用户提交下一条消息时再切回工作中。其他任务造成卡片刷新时，画面仍显示需要确认，但不会重复播放同一条输入提醒。

Codex 被强制结束时不会发送 `SessionEnd`，因此活动还必须能自行过期。`vibebuddyd` 每 60 秒扫描一次：工作中超过 30 分钟没有任何 Hook 事件即视为进程已消失，需要确认则保留 4 小时，以免用户离开期间丢失提醒。清空最后一个活动时发送标题为 `TIMED OUT` 的 `agent.idle`，不播放语音。

实机结论（2026-09-14）：临时把时限缩短到秒级后，被遗弃的工作中活动在超时后由后台扫描清除，设备回报 `TITLE TIMED OUT` 与 `DISPLAY STATE READY`，没有 `AUDIO QUEUED`；等待确认的活动跨过工作中时限的五次扫描仍然保留，直到超过自身时限才释放。固件未修改，也未重新烧录。

Codex Hook 没有直接提供“这条助手回复是否要求用户回答”的结构化字段。`Stop` 会提供 `last_assistant_message`，隐私过滤脚本仅在本机检查最后一段是否包含明确问题或回复指令，然后生成 `response_kind: input_required`。这是保守的文本规则，不是对回复正文做远端语义分析。

两个 Adapter 共用的英文等待判定会忽略客套结尾中的 Markdown 强调标记，以及不含文字内容的尾随标点或表情：`**Let me know** if you need anything else.`、`Just let me know…` 和 `Just let me know :)` 不要求输入；带格式的明确请求仍要求输入。

## K2 导航与运行处

K2 的落点取决于运行处，规则与 Claude Code 共用一套（见 [`claude-adapter.md`](claude-adapter.md#k2-导航)）：

| 运行处 | 判据 | K2 打开 |
| --- | --- | --- |
| Codex App | `__CFBundleIdentifier` 是 `com.openai.codex` | `codex://threads/<thread_id>` |
| 别的应用（终端、编辑器） | `__CFBundleIdentifier` 是别人 | `open -b <那个 bundle id>` |
| 没有宿主（SSH、后台进程） | 没有 `__CFBundleIdentifier` | 跳过，试下一个候选 |

Codex App 装在 `ChatGPT.app` 里，`codex:` scheme 也由那个 bundle 认领（2026-09-21 用 `lsregister` 核对）。

Codex 自己注入 `CODEX_SESSION_ID` 与 `CODEX_THREAD_ID`，但它们对判定运行处没用——App 里的会话大概率也有。判定只看 `__CFBundleIdentifier`。

Codex 没有 `CLAUDE_CODE_HOST_SESSION_ID` 那样的第二身份，因此没有办法纠正"跑在 Codex App 自己的终端面板里"这种情形：它会被判成 App 会话。后果有限——K2 仍然把 Codex App 拉到前台，只是落在 thread 视图而不是那个面板。

## 隐私边界

Hook 的原始 JSON 可能含 prompt、transcript 路径、工具输入和工具输出。`tools/codex-hook.py` 在发送 HTTP 前只保留：

- `session_id`
- `turn_id`
- `thread_id`（K2 导航目标；顶层会话等于 `session_id`，子 Agent 映射到拥有它的父会话）
- `hook_event_name`
- `cwd`

当且仅当 `Stop` 被本机规则判定为等待回答时，脚本还会加入派生字段 `response_kind`。`last_assistant_message` 本身不会发送给 daemon。

此外还有两个来自进程环境、不含用户内容的派生字段：`surface` 与 `host_bundle_id`（见上节）。

它只请求 `http://127.0.0.1:7331/v1/codex-hooks`，超时为 0.5 秒；daemon 未运行、载荷无效或连接失败时静默退出 0，不阻塞 Codex。不要改成直接 `curl --data-binary @-`，否则敏感字段会越过适配器边界。

## 安装与信任

用户级 `~/.codex/hooks.json` 为上述六个事件调用：

```text
/usr/bin/python3 /Users/dragon/workspace/vibe-buddy/tools/codex-hook.py
```

Hook 配置新增或变更后，Codex 会按精确定义哈希要求重新审查。打开 `/hooks`，核对脚本路径与六个事件后再信任；不要绕过信任机制。新会话或重新加载后的 Codex 才会使用新配置。

Codex 的子 Agent 有自己的生命周期 `session_id`，但这个内部线程不一定能被桌面端 deeplink 打开。Hook 脚本只读取 transcript 第一行的 `session_meta`，在本机把 `source.subagent.thread_spawn.parent_thread_id` 派生为 `thread_id`；正文和 transcript 路径都不会发给 `vibebuddyd`。活动去重仍使用子 Agent 的 `session_id + turn_id`，只有 K2 导航使用父线程，不能混用这两个身份。

## 后台运行

## 后台会话

Codex 会为自己的后台会话触发同一套 Hook，典型的是每个回合结束后生成 ambient suggestions 的那次运行：它没有工作目录，`~/.codex/state_5.sqlite` 的 `threads` 表里也没有它的 id。2026-09-16 实测它紧跟在一个真回合之后结束，于是设备连播两次"任务完成"，K2 又把它当作最近播报的那件事，打开 `codex://threads/<id>` 得到一个空白会话。

`vibebuddyd` 因此在入口处过滤：没有可用工作目录、且线程表里查不到的 Codex 会话一律忽略（记一行 info），不出卡片、不播报、不做 K2 落点。两个条件缺一不可——刚建立的真会话可能还没来得及写进线程表，但它有工作目录。K2 打开前还会再核对一次线程存在，不存在就跳到下一个候选。`tools/codex-hook.py` 另在 `~/Library/Logs/VibeBuddy/codex-hooks.log` 留一行不含 prompt 的诊断记录（事件名、会话与线程 id、目录、transcript 有无），下次再冒出来路不明的会话时用它看 Hook 到底收到了什么。

`vibebuddyd` 的 release binary 由 `~/Library/LaunchAgents/com.vibebuddy.vibebuddyd.plist` 在登录后自动启动，并在异常退出后重启。日志写入 `~/Library/Logs/VibeBuddy/vibebuddyd.log`。USB 设备暂时不存在时进程保持运行并定期重新发现，Hook 不需要感知拔插。

更新 daemon 后执行 `cargo build --release -p vibebuddyd`，再用 `launchctl kickstart -k gui/502/com.vibebuddy.vibebuddyd` 重启服务。烧录固件前先停止该服务，避免它占用串口。

## 为什么不抓 UI 或 transcript

- UI 像素和可访问性树不是生命周期协议，版本更新会改变。
- Codex 官方说明 transcript 格式不是 Hook 的稳定接口。
- Hook 直接给出语义事件，数据更少，延迟更低，也更容易测试。

未来若 Codex App Server 提供可附着的稳定桌面会话流，可增加第二种适配器以区分 `systemError`、等待审批和等待输入；当前桌面进程使用 stdio 子进程，不能假定外部 daemon 可以附着。

## vibebuddy-hook

2026-09-16 起 Hook 由 Rust 二进制 `vibebuddy-hook codex` 处理（源码在 `hook/`），它的前身 `tools/codex-hook.py` 已退役，测试用例逐条搬了过去。App 把它复制到 `~/Library/Application Support/VibeBuddy/bin/vibebuddy-hook`，配置里的命令写作 `"<那个路径>" codex`；手工配置时也用这个路径，别指向 App 包内，App 挪位置会断（ADR-0005）。
