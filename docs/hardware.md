# AgentBeacon 硬件记录

本文只记录带证据边界的硬件事实。相似名称或同系列开发板不能作为 GPIO、codec、LCD 或 USB 路径的依据。

## 用户提供，尚未由实物或原理图验证

- 名称：正点原子 ESP32S3-BOX。
- 模组：ATK-MWS3S / ESP32-S3。
- 资源：16 MB Flash、8 MB PSRAM。
- 外设：LCD、Speaker、Microphone、Buzzer、K0/K1/K2、TF/microSD、USB-C、USB-A Host、UART。
- 期望：一根 USB-C 线同时供电、烧录和运行时通信。

同一根 USB-C 线完成供电、运行时串口和 ROM 下载链路已经实测成立；实际 AgentBeacon 固件写入及写入后的重新枚举仍待 Stage 1 验收。

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

因此，在 PCB 丝印确认前不选择 BSP、不冻结 GPIO。ESP-IDF v5.5.3 已根据实机当前固件的 v5.5 构建信息和对应上游源码要求安装；该选择不代表已经接受某个候选板的 GPIO 定义。

## 实机确认

- 用户确认开发板尚未连接后，已于 2026-09-13 保存 Mac Studio 的未连接 USB 基线；基线未出现 ESP32、常见 USB-UART bridge 或新增 USB modem 串口。
- 2026-09-14 连接开发板后，新增 Espressif `USB JTAG/serial debug unit`，VID:PID `303A:1001`，USB serial `98:88:E0:06:8B:CC`。
- 新增节点为 `/dev/cu.usbmodem8401` 与 `/dev/tty.usbmodem8401`，证明目标板当前通过 ESP32-S3 原生 USB Serial/JTAG 枚举，而不是外置 CH340/CP210/FTDI bridge。
- 运行日志确认 ESP32-S3 revision v0.2、16 MB QIO Flash、8 MB Octal PSRAM，以及当前固件板型标识 `atk-dnesp32s3-box`。
- 只读 `esptool flash_id` 成功连接 ROM 下载通路并复核 16 MB Flash、8 MB embedded PSRAM 和 USB-Serial/JTAG mode；未擦除或写入 Flash。
- 打开原生 USB 串口会触发 `USB_UART_CHIP_RESET`，因此运行时重连设计必须容忍设备复位和重新枚举。

## 当前运行固件旁证

实机运行 `xiaozhi` 1.9.4。固定版本源码中存在 `atk-dnesp32s3-box` board 目录，并记录了 ST7789 i80、XL9555、ES8311 和一组板级引脚。这与启动日志相互印证，可作为核对候选，但该仓库不是正点原子官方原理图/BSP，不能越过 PCB 版本门禁直接冻结引脚。

- [`xiaozhi-esp32` v1.9.4 固定提交](https://github.com/78/xiaozhi-esp32/tree/3ced7709c65a39494f5684e99111854a5bcbd8c7)
- [`atk-dnesp32s3-box/config.h`](https://github.com/78/xiaozhi-esp32/blob/3ced7709c65a39494f5684e99111854a5bcbd8c7/main/boards/atk-dnesp32s3-box/config.h)

## 待验证

- PCB 丝印中的完整型号和硬件版本。
- 所有 USB 口、UART、HOST、OTG、SLAVE、DOWNLOAD/JTAG 的实物丝印。
- 与准确 PCB 版本匹配的厂家 schematic、BSP 和 examples。
- AgentBeacon 固件实际烧录后的枚举、复位和运行时串口路径。
- LCD 控制器、分辨率和接线。
- 触摸控制器（如有）。
- audio codec、功放和麦克风接口。
- buzzer 与 K0/K1/K2 的 GPIO 和有效电平。
