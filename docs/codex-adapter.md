# Codex 实时适配器

## 能力边界

Codex 内置宠物的素材、动画状态机和任务卡片渲染没有公开为宠物 API，不能可靠地逐帧镜像到外部设备。可依赖的公开接口是 Codex 生命周期 Hook。

AgentBeacon 使用以下映射：

| Codex Hook | AgentBeacon 状态 |
| --- | --- |
| `UserPromptSubmit` | 工作中 |
| `PermissionRequest` | 需要确认 |
| `PostToolUse` | 恢复工作中 |
| `Stop`（回复正在等待用户回答） | 需要确认 |
| `Stop`（其他回复） | 完成 |
| `Interrupt` | 空闲，标题为 `INTERRUPTED`，不误报成功 |
| `SessionEnd` | 空闲 |

多会话由 `beacond` 聚合：任务卡按最近活动排序，最多 3 张；需要确认的会话优先控制宠物表情。活动身份由 `session_id` 与 `turn_id` 共同确定。每个已跟踪、且不等待用户回答的 turn，其 `Stop` 都会产生一次完成通知，即使画面仍需显示其他工作中的任务；同一 turn 的重复 `Stop` 不会重复播报。等待回答的 turn 保持为需要确认，用户提交下一条消息时再切回工作中。其他任务造成卡片刷新时，画面仍显示需要确认，但不会重复播放同一条输入提醒。

Codex 被强制结束时不会发送 `SessionEnd`，因此活动还必须能自行过期。`beacond` 每 60 秒扫描一次：工作中超过 30 分钟没有任何 Hook 事件即视为进程已消失，需要确认则保留 4 小时，以免用户离开期间丢失提醒。清空最后一个活动时发送标题为 `TIMED OUT` 的 `agent.idle`，不播放语音。

Codex Hook 没有直接提供“这条助手回复是否要求用户回答”的结构化字段。`Stop` 会提供 `last_assistant_message`，隐私过滤脚本仅在本机检查最后一段是否包含明确问题或回复指令，然后生成 `response_kind: input_required`。这是保守的文本规则，不是对回复正文做远端语义分析。

## 隐私边界

Hook 的原始 JSON 可能含 prompt、transcript 路径、工具输入和工具输出。`tools/codex-hook.py` 在发送 HTTP 前只保留：

- `session_id`
- `turn_id`
- `hook_event_name`
- `cwd`

当且仅当 `Stop` 被本机规则判定为等待回答时，脚本还会加入派生字段 `response_kind`。`last_assistant_message` 本身不会发送给 daemon。

它只请求 `http://127.0.0.1:7331/v1/codex-hooks`，超时为 0.5 秒；daemon 未运行、载荷无效或连接失败时静默退出 0，不阻塞 Codex。不要改成直接 `curl --data-binary @-`，否则敏感字段会越过适配器边界。

## 安装与信任

用户级 `~/.codex/hooks.json` 为上述六个事件调用：

```text
/usr/bin/python3 /Users/dragon/workspace/agent-beacon/tools/codex-hook.py
```

Hook 配置新增或变更后，Codex 会按精确定义哈希要求重新审查。打开 `/hooks`，核对脚本路径与六个事件后再信任；不要绕过信任机制。新会话或重新加载后的 Codex 才会使用新配置。

## 后台运行

`beacond` 的 release binary 由 `~/Library/LaunchAgents/com.agentbeacon.beacond.plist` 在登录后自动启动，并在异常退出后重启。日志写入 `~/Library/Logs/AgentBeacon/beacond.log`。USB 设备暂时不存在时进程保持运行并定期重新发现，Hook 不需要感知拔插。

更新 daemon 后执行 `cargo build --release -p beacond`，再用 `launchctl kickstart -k gui/502/com.agentbeacon.beacond` 重启服务。烧录固件前先停止该服务，避免它占用串口。

## 为什么不抓 UI 或 transcript

- UI 像素和可访问性树不是生命周期协议，版本更新会改变。
- Codex 官方说明 transcript 格式不是 Hook 的稳定接口。
- Hook 直接给出语义事件，数据更少，延迟更低，也更容易测试。

未来若 Codex App Server 提供可附着的稳定桌面会话流，可增加第二种适配器以区分 `systemError`、等待审批和等待输入；当前桌面进程使用 stdio 子进程，不能假定外部 daemon 可以附着。
