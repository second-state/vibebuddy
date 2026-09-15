# Beacon Protocol v1

Beacon Protocol 是 `beacond` 与 Vibe Buddy 设备之间的应用协议。v1 使用 UTF-8 NDJSON；transport 负责可靠地传送字节流，协议不依赖具体串口名称。

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
- `device.heartbeat`：证明链路存活，不显示、不回显诊断行、不播放语音。

`beacond` 可附加最多 3 项的 `tasks` 数组。数组按最近活动倒序，设备按给定顺序绘制任务卡：

```json
{"version":1,"event":"task.start","title":"GAMMA","tasks":[{"title":"GAMMA","status":"working","elapsed_s":75},{"title":"BETA","status":"input_required","elapsed_s":900}],"stats":["7 DONE","4 ASKS","1H23 BUSY"]}
```

每项包含 `title`、`status` 和 `elapsed_s`；当前状态值为 `working`、`input_required`、`done`、`failed`。旧固件会按 v1 规则忽略 `tasks`。未来事件可以包括 `task.progress`、`task.cancelled`、`agent.waiting`、`message`、`system` 和 `device.status`。

`elapsed_s` 是该活动进入**当前状态**已经过去的秒数，不是距上一个事件的秒数：工作中的卡片回答「这个 turn 跑了多久」，等待确认的卡片回答「等了多久」。设备收到后自行继续计时，因为可见状态不变时 `beacond` 会去重、不再发消息，而屏幕上的数字必须一直走。也正因为要去重，`elapsed_s` 与 `stats` 都在去重之后才盖到事件上；放进快照会让每个工具事件都变成一次重绘，把工作中的动画不断打回第一帧。

`stats` 是最多 3 行当日战绩，空闲屏轮播它们。它随每条状态事件下发而不只随 `agent.idle`：`task.done` 之后设备是自己回到空闲的，那一刻正是用户会看的一眼，缓存的战绩必须已经包含刚刚完成的那一件。计数按本地自然日归零，并持久化到 `~/Library/Application Support/AgentBeacon/stats.json`，否则每次重启 daemon 屏幕上写着「今天」的数字都会归零。

当一个后台任务完成、但画面仍需显示其他活动任务时，`beacond` 会在当前状态事件上附加 `"announcement":"done"`。这是一次性语音通知，不改变画面状态；`announcement_id` 用于标识对应 turn。设备收到它时排队播放一次“任务完成”。失败走同一条路径，取值为 `"announcement":"failed"`，播放一次“任务遇到问题”。

当聚合任务变化仅需重绘既有的输入等待状态时，`beacond` 会附加 `"suppress_audio":true`。设备继续显示 `agent.input_required`，但不重复播放已经播过的提醒。`announcement` 的一次性通知优先于此字段。

`beacond` 每 5 秒发送一次心跳，并捎上自己的构建标识、本地小时数与本地日期：

```json
{"version":1,"event":"device.heartbeat","build":"9b642af 2026-09-14 17:41","hour":14,"day":20260915}
```

`build` 是 `git describe --always --tags --dirty` 加上二进制的时间戳。它随心跳重复发送而不是握手一次，因为设备可能随时重启，一次性的握手会丢。设备只在取值变化时才重绘，否则每 5 秒就要刷一次屏。`hour` 是 Mac 端的本地小时，休闲模式用它区分白天黑夜；`day` 是本地日期 YYYYMMDD，番茄钟的当日记录按它清零。设备没有时钟，也不该为了这个去连 Wi-Fi。旧 daemon 不发它们时设备当白天处理、不换日。

设备只显示它，不拿它和自己的固件标识比对：两边的发布节奏本来就不同步，把不一致当告警只会制造持续的假警报。字段缺失时设备显示 `?`，这说明对面是个还不发这个字段的旧 daemon。

设备超过 15 秒没有收到**任何**消息即判定失联，覆盖显示为 `NO LINK` 并把画面转灰；收到任何一行合法消息即恢复。判定依据是所有消息而不只是心跳，因为繁忙时真实事件本身就足以证明链路存活。固件不为心跳输出 `EVENT` 诊断行，否则每天会产生上万行日志。

设备到 Mac 的事件同样使用 NDJSON，例如：

```json
{"version":1,"event":"button","button":"K2","action":"press"}
```

当前只上报 K2 短按，在任何模式里都上报。K0（番茄钟）与 K1（切换模式、长按去休闲）由固件自己消费，不上报。短按在松开时才算数，因为只有等到松开才知道它不是长按的开头；上报时机因此从按下推迟到松开。模式与番茄钟见 [`pomodoro.md`](pomodoro.md)，休闲见 [`leisure.md`](leisure.md)。`beacond` 收到后按这个顺序选落点：等人回答的任务优先（屏幕主状态显示的就是它，而且它 blocking 着人）；其次是最近一次播报过结束的任务——它已经离开卡片栈，屏幕上再也看不到，而还在跑的任务一直挂在屏幕上、本来就不需要 K2 定位；再次才是最新工作项。落点由下一次播报接力替换，不设时间窗：这台设备的用处正是人不在电脑前，用墙上时钟让落点过期，等于假设用户一直守在旁边。选定活动后，Codex 顶层活动打开自身 thread，子 Agent 活动打开拥有它的父 thread；Claude Code 先把 Hook 报的 CLI `session_id` 连同 `cwd` 在 Claude App 的会话索引里解析成桌面会话 id，再用 `claude://code/continue?session=<桌面会话 id>` 精确跳转；解析不到（会话只跑在终端里）才退回 `claude://resume?session=<session_id>` 导入；GitHub Actions 打开对应 run。当前没有活动时回到最近一次可定位的来源；该定位会写入本机状态文件，daemon 重启后仍然有效。K0、K1 和长按/释放尚未绑定。

固件输出的 `READY`、`EVENT`、`TITLE` 和 `ERROR` 行是实机链路验收用的诊断文本，不是设备到 Mac 的正式 JSON 事件。同类的还有 `MODE DUTY` / `POMODORO` / `LEISURE`（模式切换），`POMODORO FOCUS START` / `FOCUS END` / `BREAK START` / `BREAK END` / `PAUSED` / `RESUMED` / `STOPPED` / `BREAK SKIPPED`（番茄钟转换），`LEISURE ALERT` / `BORED` / `SLEEPY`、`LEISURE SKIT <名>`、`LEISURE LIGHTS OUT` / `ON`（休闲），以及 `CLOCK HOUR <n>`（收到的小时数变化）、`TALLY LOADED <次> <秒>S DAY <日期>`（开机恢复的当日记录）。番茄钟与休闲的状态都只在固件里，Mac 端只记日志。

## Stage 1 错误输出

| 输出 | 含义 |
| --- | --- |
| `ERROR invalid_json` | 该行不是完整合法的 JSON |
| `ERROR invalid_message` | 根不是 object，或缺少/误用 `version`、`event`、`title` |
| `ERROR unsupported_version` | `version` 不是 `1` |
| `ERROR input_too_large` | 一行超过 1024 bytes |
