# AgentBeacon 路线图

## 阶段门禁

### Stage 0 — Hardware Probe（已完成，2026-09-14）

完成条件：

- 当前开发主机确认为 Mac Studio。
- 记录开发工具路径与版本，明确缺失项和安装方案。
- 保存 ESP32S3-BOX 插拔前后的 USB、IOKit 和串口枚举，并形成差异。
- 确认 VID/PID、设备名、串口节点、USB 类型、烧录路径和运行时路径，或把尚不能证明的项目明确标记为待验证。
- 精确识别 PCB 型号/版本，并调查匹配的官方 schematic、BSP 和 examples；无法取得的资料必须记录证据边界，并继续锁定依赖它们的外设阶段。

最小 USB/烧录路径未确认前，Stage 0 不通过。

实机结论：ATK-DNESP32S3-BOX V1.1 通过 `303A:1001` 原生 USB Serial/JTAG 枚举；同一 `USB-SLAVE` 连接已完成 ROM 探测、烧录和运行时通信。老款 BOX V1.1 的厂家原理图/BSP 仍未取得，因此 Stage 3～5 的外设 GPIO 不得按相似板卡猜测。

### Stage 1 — Serial Hello（已完成，2026-09-14）

只实现串口逐行读取和 JSON 解析，不实现 LCD/audio。验收必须是 Mac → USB → ESP32 → JSON parse 的实机链路，不能用本机模拟或仅编译通过代替。

实机结论：ESP-IDF v5.5.3 构建和烧录成功；Mac 发送 `{"version":1,"event":"task.done","title":"Hello"}` 后，ESP32 实际返回 `EVENT task.done` 与 `TITLE Hello`。原厂 16 MB Flash 已在写入前完成本地受限权限备份。

### Stage 2 — `beacond`（已完成，2026-09-14）

实现 `POST /v1/events`、最小 Transport 抽象、`SerialTransport` 和断线重连；用 HTTP 请求完成实机验收。

实机结论：`beacond` 按 `303A:1001` 自动发现 `/dev/cu.usbmodem8401`；HTTP 事件到达 ESP32 并得到对应诊断输出。真实拔掉 `USB-SLAVE` 后记录到 `Device not configured`，插回后自动重新连接；重连后的 `task.done` 事件成功到达设备。

### Stage 3 — LCD（已完成，2026-09-14）

实机确认 320×240 ST7789 i80 与 XL9555 背光控制后，实现小灯灵的 Ready、Working、Input Required、Done、Failed 动画。Mac 端多任务快照可绘制最多 3 张卡片，最新在最上。

### Stage 4 — Audio（已完成，2026-09-14）

实机探测到 ES8311，并确认 I2S 与扬声器使能链路。需要确认、完成和失败使用固件内置的 24 kHz PCM 中文短语音；工作中与空闲保持静音。用户已听觉确认“需要你确认”。

### Stage 5 — Buttons（K2 已完成，2026-09-14）

K2“打开当前来源”的按键与 Mac 激活链路已经通过实机验收：探针确认 K2 为 XL9555 P0.3、低电平有效（`P0: 0xFF → 0xF7 → 0xFF`）；设备通过 NDJSON 上报单击，用户短按后 `beacond` 能拉起目标应用。精确路由已修正两处：Codex 子 Agent 映射到父 thread；Claude Code 不再聚焦 Ghostty，而是打开 Claude App 的对应 Code 会话。2026-09-15 修正了其中的定位错误：CLI `session_id` 到桌面会话不是一对一，原先的 `claude://resume` 会打开一个内容陈旧的影子会话，现改为先解析桌面会话 id 再用 `claude://code/continue` 跳转。两条 deeplink 均已单独验证，完整 K2 页面跳转仍需一次实机短按确认。

2026-09-15 加入第二个场景：番茄钟（专注 25 分钟、休息 5 分钟，表盘仿 Focus To-Do）。三个键各管一件事：K0 短按开始/暂停/继续、长按放弃，K1 切换场景，K2 照旧打开来源；阶段结束播钟声加语音并自动切到番茄钟，下一阶段等用户按 K0 才开始。状态机有主机测试，画面有主机预览。同日经 UART 桥烧录后实机确认三个键都工作：K0 为 GPIO0，K1 由候选位探针定为 XL9555 P0.4，K2 照旧打开来源；场景切换、开始、暂停、继续均有设备回报。第一次实机也暴露出短促轻点会被 100 ms 的采样加两次一致去抖丢掉，已改为 20 ms 采样、翻转即生效。专注结束与休息结束的钟声、语音和自动切场景尚待一次完整的 25 分钟实机验收。设计见 [`pomodoro.md`](pomodoro.md)。

Stage 5 整体尚未完成：K2 在无活动/已完成状态下的扩展行为仍未实现；原计划的 K0 静音已让位给番茄钟。

### Stage 6 — `beacon`

实现 `start`、`done`、`error` 和 `run`；CLI 只调用 `beacond`。

## Later

可选任务标题、microphone、voice interaction、长期任务历史、Wi-Fi、WebSocket transport、progress、多设备。多 Agent 的三任务实时卡片已提前进入 Stage 4。
