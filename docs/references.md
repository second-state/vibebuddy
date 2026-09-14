# 实现参考与适用边界

本文记录可借鉴的外部实现，不把参考项目当成 AgentBeacon 硬件事实来源。

## `second-state/echokit_box`

核查版本：[`4484efca885c2ffd01ffb1acdbb5817421583bd8`](https://github.com/second-state/echokit_box/tree/4484efca885c2ffd01ffb1acdbb5817421583bd8)

可借鉴：

- Rust 固件使用 `esp-idf-svc`，并通过 `esp-idf-sys` 接入 C component；这说明“Rust 上层 + 厂家/ESP-IDF C driver”在 ESP32-S3 上是可行组织方式。[Cargo.toml](https://github.com/second-state/echokit_box/blob/4484efca885c2ffd01ffb1acdbb5817421583bd8/Cargo.toml)
- 工程把板级差异放在 board module/feature 中，适合以后确有第二种硬件时参考；AgentBeacon 第一块板不提前复制其多板抽象。
- 仓库固定 `ESP_IDF_VERSION = "v5.4.1"`，使用 `xtensa-esp32s3-espidf` 和 `espflash`。[`.cargo/config.toml`](https://github.com/second-state/echokit_box/blob/4484efca885c2ffd01ffb1acdbb5817421583bd8/.cargo/config.toml)
- README 记录 EchoKit 设备通过标为 OTG/SLAVE 的口枚举成 JTAG 串口，并给出 `/dev/cu.usbmodem...` 示例；这可作为 AgentBeacon 插板后的一个排查假设，不能当成目标板结论。[README](https://github.com/second-state/echokit_box/blob/4484efca885c2ffd01ffb1acdbb5817421583bd8/README.md)

不直接采用：

- EchoKit 的协议面向 Wi-Fi/WebSocket 音频会话，服务端事件用 MessagePack，设备命令部分用 JSON；AgentBeacon v1 是本地 USB 串行 NDJSON，目标和 framing 不同。[`src/protocol.rs`](https://github.com/second-state/echokit_box/blob/4484efca885c2ffd01ffb1acdbb5817421583bd8/src/protocol.rs)
- `atom_box.rs` 和 `components/hal_driver` 中的 GPIO、ES8311、XL9555、320×240 LCD 等参数只属于 EchoKit 对应板型。除非精确 PCB/原理图证明一致，否则不得复制到 AgentBeacon。
- 该仓库使用 GPL-3.0。没有确定 AgentBeacon 的许可证兼容策略前，只借鉴思路，不复制实现代码。

## `second-state/echokit_server`

核查版本：[`d1d976596f122976095b7da4df3e946baf152b96`](https://github.com/second-state/echokit_server/tree/d1d976596f122976095b7da4df3e946baf152b96)

可借鉴：

- 服务端采用 Rust、Tokio、Axum、Serde 和 tracing/logging 相关生态，与 `beacond` 的候选技术栈方向一致。[Cargo.toml](https://github.com/second-state/echokit_server/blob/d1d976596f122976095b7da4df3e946baf152b96/Cargo.toml)
- WebSocket I/O 使用独立消息处理循环和 channel 把 transport 与业务流水线分隔；Stage 2 设计串口收发/重连任务时可参考这个职责边界。[`src/services/ws.rs`](https://github.com/second-state/echokit_server/blob/d1d976596f122976095b7da4df3e946baf152b96/src/services/ws.rs)

不直接采用：

- EchoKit Server 是 ASR → LLM → TTS 语音平台，范围显著大于本地状态守护进程。AgentBeacon 不引入其 AI provider、VAD、音频流、MCP 或配置系统。
- 它的网络协议、重试和音频分块策略不能替代 Beacon Protocol 的版本、逐行 framing、输入上限与未知事件规则。
- 该仓库同样使用 GPL-3.0；当前阶段不复制代码。

## 对 AgentBeacon 的实际影响

1. 保持 Mac 端 Rust + Tokio/Axum 的方向，但到 Stage 2 才创建依赖与代码。
2. 固件仍优先评估 ESP-IDF C；是否采用 Rust 固件必须以官方 BSP 可复用程度、构建复杂度和 Stage 1 最小链路为依据，不因参考仓库使用 Rust 就自动选择 Rust。
3. 插板后重点观察 `/dev/cu.usbmodem*`、USB Serial/JTAG 与多 USB 口角色，但不预设结果。
4. 许可证策略未确定前，参考仓库只用于架构比较和排障线索。

## 当前设备运行固件：`78/xiaozhi-esp32`

核查版本：[`v1.9.4 / 3ced7709c65a39494f5684e99111854a5bcbd8c7`](https://github.com/78/xiaozhi-esp32/tree/3ced7709c65a39494f5684e99111854a5bcbd8c7)

2026-09-14 的实机启动日志自报应用 `xiaozhi` 1.9.4、ESP-IDF v5.5 和板型 `atk-dnesp32s3-box`，与该固定源码版本相符。这个仓库因此是“当前设备运行固件”的一手实现来源，可用于理解当前可工作的板级配置和选择兼容 ESP-IDF 版本。

适用边界：它不是正点原子的厂家原理图/BSP，也没有给出用户手中 PCB 的硬件版本。其 [`config.h`](https://github.com/78/xiaozhi-esp32/blob/3ced7709c65a39494f5684e99111854a5bcbd8c7/main/boards/atk-dnesp32s3-box/config.h) 中的 GPIO、LCD 和音频参数只能作为待核对候选，不能单独成为 AgentBeacon 的最终硬件依据。
