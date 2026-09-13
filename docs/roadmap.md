# AgentBeacon 路线图

## 阶段门禁

### Stage 0 — Hardware Probe（进行中）

完成条件：

- 当前开发主机确认为 Mac Studio。
- 记录开发工具路径与版本，明确缺失项和安装方案。
- 保存 ESP32S3-BOX 插拔前后的 USB、IOKit 和串口枚举，并形成差异。
- 确认 VID/PID、设备名、串口节点、USB 类型、烧录路径和运行时路径，或把尚不能证明的项目明确标记为待验证。
- 精确识别 PCB 型号/版本，并匹配官方 schematic、BSP 和 examples。

最小 USB/烧录路径未确认前，Stage 0 不通过。

### Stage 1 — Serial Hello

只实现串口逐行读取和 JSON 解析，不实现 LCD/audio。验收必须是 Mac → USB → ESP32 → JSON parse 的实机链路，不能用本机模拟或仅编译通过代替。

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

