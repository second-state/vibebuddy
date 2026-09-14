# Beacon Protocol v1

Beacon Protocol 是 `beacond` 与 AgentBeacon 设备之间的应用协议。v1 使用 UTF-8 NDJSON；transport 负责可靠地传送字节流，协议不依赖具体串口名称。

## Framing

- 每条消息是一个 JSON object，以单个 LF（`\n`）结束。
- 接收端同时接受 CRLF（`\r\n`），但发送端统一使用 LF。
- 空行忽略。
- 单条 JSON 最多 1024 bytes，不含行结束符。超出后丢弃整行并返回 `ERROR input_too_large`。
- v1 不支持跨行 JSON、JSON array 或多个 JSON object 共用一行。

## Envelope

每条消息必须包含：

- `version`：整数；v1 必须等于 `1`。
- `event`：非空字符串。

`id`、`title`、`message` 等字段由具体事件使用。Stage 1 不要求 `id`，所以下面的 Hello 是合法消息：

```json
{"version":1,"event":"task.done","title":"Hello"}
```

接收端忽略未知字段，以便旧固件接受新增的可选字段。未知 `event` 也必须完成 framing 和 envelope 解析；Stage 1 固件会输出其名称，不因事件尚未实现而破坏连接。无法识别的 `version` 返回 `ERROR unsupported_version`，不能按 v1 猜测处理。

## 方向

Mac 到设备的首批事件：

- `task.start`
- `task.done`
- `task.error`

当前固件识别：

- `task.start`：工作中，不播放语音。
- `agent.input_required`：需要用户确认，播放一次“需要你确认”。
- `task.done`：完成，播放一次“任务完成”，5 秒后回到空闲。
- `task.error` / `agent.blocked`：失败，播放一次“任务遇到问题”。
- `agent.idle`：回到空闲，不播放语音。

`beacond` 可附加最多 3 项的 `tasks` 数组。数组按最近活动倒序，设备按给定顺序绘制任务卡：

```json
{"version":1,"event":"task.start","title":"GAMMA","tasks":[{"title":"GAMMA","status":"working"},{"title":"BETA","status":"input_required"}]}
```

每项包含 `title` 和 `status`；当前状态值为 `working`、`input_required`、`done`、`failed`。旧固件会按 v1 规则忽略 `tasks`。未来事件可以包括 `task.progress`、`task.cancelled`、`agent.waiting`、`message`、`system` 和 `device.status`。

设备到 Mac 的事件同样使用 NDJSON，例如：

```json
{"version":1,"event":"button","button":"K0","action":"press"}
```

Stage 1 固件输出的 `READY`、`EVENT`、`TITLE` 和 `ERROR` 行是实机链路验收用的诊断文本，不是设备到 Mac 的正式 JSON 事件。按钮事件到 Stage 5 才实现。

## Stage 1 错误输出

| 输出 | 含义 |
| --- | --- |
| `ERROR invalid_json` | 该行不是完整合法的 JSON |
| `ERROR invalid_message` | 根不是 object，或缺少/误用 `version`、`event`、`title` |
| `ERROR unsupported_version` | `version` 不是 `1` |
| `ERROR input_too_large` | 一行超过 1024 bytes |
