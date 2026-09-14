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

### Stage 2 — `beacond`

实现 `POST /v1/events`、最小 Transport 抽象、`SerialTransport` 和断线重连；用 HTTP 请求完成实机验收。

### Stage 3 — LCD

在官方参数/BSP 经确认后实现 Ready、Working、Done、Failed 状态。

### Stage 4 — Buzzer / Audio

先实现最简单可靠的成功和失败提示音，不做 TTS。

### Stage 5 — Buttons

实现 K0 静音、K1 最近事件、K2 acknowledge，并向 Mac 上报按钮事件。

### Stage 6 — `beacon`

实现 `start`、`done`、`error` 和 `run`；CLI 只调用 `beacond`。

## Later

TTS、microphone、voice interaction、task history、Wi-Fi、WebSocket transport、richer UI、progress、多 Agent、多设备。
