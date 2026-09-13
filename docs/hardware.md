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

等待精确板型和版本的官方资料核查。研究记录将单独保存，并把可映射到实物的结论回填到这里。

## 实机确认

- 用户确认开发板尚未连接后，已于 2026-09-13 保存 Mac Studio 的未连接 USB 基线。
- 基线未出现 ESP32、常见 USB-UART bridge 或新增 USB modem 串口。
- 尚未采集开发板连接后的快照，因此 USB 插拔差异仍未完成。

## 待验证

- PCB 丝印中的完整型号和硬件版本。
- USB VID/PID、产品名和序列号。
- `/dev/cu.*` 与 `/dev/tty.*` 节点。
- 是原生 USB、USB Serial/JTAG、USB CDC，还是 USB-UART bridge。
- 烧录与运行时通信是否经过同一物理接口和同一设备节点。
- LCD 控制器、分辨率和接线。
- 触摸控制器（如有）。
- audio codec、功放和麦克风接口。
- buzzer 与 K0/K1/K2 的 GPIO 和有效电平。
