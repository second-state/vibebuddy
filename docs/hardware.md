# AgentBeacon 硬件记录

本文只记录带证据边界的硬件事实。相似名称或同系列开发板不能作为 GPIO、codec、LCD 或 USB 路径的依据。

## 用户提供，尚未由实物或原理图验证

- 名称：正点原子 ESP32S3-BOX。
- 模组：ATK-MWS3S / ESP32-S3。
- 资源：16 MB Flash、8 MB PSRAM。
- 外设：LCD、Speaker、Microphone、Buzzer、K0/K1/K2、TF/microSD、USB-C、USB-A Host、UART。
- 期望：一根 USB-C 线同时供电、烧录和运行时通信。

最后一项是待验证假设，不是当前结论。

## 当前主机确认

- 主机名：`Michaels-Mac-Studio.local`。
- 型号：Mac Studio，Model Identifier `Mac14,14`，Apple M2 Ultra。
- 证据：2026-09-13 在本机执行 `hostname` 和 `system_profiler SPHardwareDataType`。

## 官方资料确认

详细来源与候选板差异见 [`hardware-research.md`](hardware-research.md)。当前能够确认的是：

- `ATK-MWS3S` 是模组标识，不足以识别底板；正点原子的 DNESP32S3 开发板和老款 ESP32S3 BOX 都可能使用该模组。
- 官方 `ATK-DNESP32S3-Board` 仓库对应 DNESP32S3 开发板，不是 BOX 的通用 BSP。它公开的是 `ATK_DNESP32S3 V1.2` 原理图、KEY0～KEY3 + BOOT、ES8388、XL9555、CH340C 和原生 USB Serial/JTAG 两条烧录路径。
- 当前官方 Wiki 完整覆盖的是 `ATK-DNESP32S3B3 V1`（BOX3）：K0 直连 GPIO0，K1/K2 经过 AW9523B，屏幕为 320×240 ST7789V2，触摸为 CHSC5432，音频链路包含 ES8311、ES7210 与 NS4150B，并公开原生 USB/TinyUSB 资料。
- 用户给出的硬件组合与上述任一候选都不完全闭合。老款 BOX 的官方资料入口当前无法读取，不能拿 BOX3 或 DNESP32S3 开发板引脚补齐。

因此，在 PCB 丝印确认前不选择 BSP、不冻结 GPIO，也不安装由某一候选示例决定的 ESP-IDF 版本。

## 实机确认

- 用户确认开发板尚未连接后，已于 2026-09-13 保存 Mac Studio 的未连接 USB 基线。
- 基线未出现 ESP32、常见 USB-UART bridge 或新增 USB modem 串口。
- 尚未采集开发板连接后的快照，因此 USB 插拔差异仍未完成。

## 待验证

- PCB 丝印中的完整型号和硬件版本。
- 所有 USB 口、UART、HOST、OTG、SLAVE、DOWNLOAD/JTAG 的实物丝印。
- USB VID/PID、产品名和序列号。
- `/dev/cu.*` 与 `/dev/tty.*` 节点。
- 是原生 USB、USB Serial/JTAG、USB CDC，还是 USB-UART bridge。
- 烧录与运行时通信是否经过同一物理接口和同一设备节点。
- LCD 控制器、分辨率和接线。
- 触摸控制器（如有）。
- audio codec、功放和麦克风接口。
- buzzer 与 K0/K1/K2 的 GPIO 和有效电平。
