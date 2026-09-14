# Stage 0 — Hardware Probe 记录

更新时间：2026-09-14（Asia/Singapore）

## 当前结论

Stage 0 **尚未完全通过**。开发主机、USB 插拔差异、运行时串口和 ROM 下载链路已经实机确认；芯片、Flash/PSRAM 容量以及当前运行固件的板型标识也已确认。剩余门禁是 PCB 丝印中的准确硬件版本，以及与该版本匹配的厂家原理图/BSP。未取得这些依据前，不冻结外设 GPIO，也不进入 LCD、音频或按键实现。

Stage 1 所需的最小 USB 前置条件已经满足：同一根 USB-C 线可供电、读取运行时日志，并让 `esptool` 进入 ESP32-S3 ROM 下载链路。这里的“下载链路已确认”不等于“AgentBeacon 固件已烧录”；本阶段没有擦除或写入 Flash。

## 主机身份

| 项目 | 实测结果 |
| --- | --- |
| Hostname | `Michaels-Mac-Studio.local` |
| Model Name | Mac Studio |
| Model Identifier | `Mac14,14` |
| Chip | Apple M2 Ultra |
| Architecture | `arm64` |
| macOS | 26.6.2（25G83） |

主机满足“只在 Mac Studio 创建项目”的前置条件。

## 开发环境

### 初始探测

| 工具 | 初始状态 | 初始版本/路径 | 对当前阶段的影响 |
| --- | --- | --- | --- |
| Homebrew | 已安装 | 6.0.22，`/opt/homebrew/bin/brew` | 满足安装依赖所需条件 |
| Python 3 | 已安装 | 3.14.5，`/opt/homebrew/bin/python3` | 满足 ESP-IDF 6.x 官方最低 Python 3.10 要求 |
| CMake | 未安装 | 不在 PATH，Homebrew 无 installed keg | 固件构建前必须补齐 |
| Ninja | 未安装 | 不在 PATH，Homebrew 无 installed keg | EIM/ESP-IDF 前置依赖，必须补齐 |
| Rust | 已安装 | 1.98.0，`/Users/dragon/.cargo/bin/rustc` | 满足后续 Mac daemon 开发 |
| Cargo | 已安装 | 1.98.0，`/Users/dragon/.cargo/bin/cargo` | 满足后续 Rust workspace 构建 |
| ESP-IDF / `idf.py` | 未发现 | PATH、`~/esp`、`~/.espressif` 常见位置均未找到 | Stage 1 固件构建前必须安装 |
| `esptool` / `esptool.py` | 未发现 | 命令与当前 Python 环境均未找到 | 识别芯片/烧录前必须由选定 IDF 环境补齐 |

### 已执行安装与验证

按 Espressif 当前 macOS 官方安装路线，已通过 Homebrew 安装：

- CMake 4.4.3
- Ninja 1.13.2
- dfu-util 0.11
- libslirp 4.9.4
- ESP-IDF Installation Manager（EIM）0.19.0

安装 EIM 时，Homebrew 自动从 6.0.22 更新到 7.0.0，并要求显式信任第三方 tap。仅对 Espressif 官方 `espressif/eim` tap 建立了信任；没有处理或信任其他 tap。

实机日志显示当前固件由 ESP-IDF v5.5 构建；同一固件的 `xiaozhi` v1.9.4 源码要求 ESP-IDF 5.4 或以上。基于这两条依据，已用 EIM 安装固定稳定版 ESP-IDF v5.5.3，而不是追踪 `master` 或切换到 6.x。

安装过程出现 `compote cooking` 命令缺失及 component cache 下载失败警告，因此没有把安装器的成功提示直接当作验收。重新激活环境后独立验证结果如下：

| 工具 | 实测版本/路径 |
| --- | --- |
| ESP-IDF | v5.5.3，`/Users/dragon/.espressif/v5.5.3/esp-idf` |
| `idf.py` | ESP-IDF v5.5.3 |
| `esptool` | v4.12.0 |
| Xtensa GCC | 14.2.0_20251107 |
| IDF CMake | 3.30.2 |
| IDF Ninja | 1.12.1 |

EIM 在仓库根目录生成了包含本机绝对路径的 `eim_config.toml`。它是本机安装状态，不是项目构建配置，已加入 `.gitignore`，不提交到仓库。

官方依据：

- [ESP-IDF v6.0 macOS 安装说明](https://docs.espressif.com/projects/esp-idf/en/stable/esp32p4/get-started/macos-setup.html)
- [EIM 官方说明与 macOS Homebrew 安装方式](https://docs.espressif.com/projects/idf-im-ui/en/latest/)
- [EIM 前置依赖](https://docs.espressif.com/projects/idf-im-cli/en/latest/prerequisites.html)
- [ESP-IDF v5.5.3 官方 release](https://github.com/espressif/esp-idf/releases/tag/v5.5.3)

## USB 未连接基线

用户确认开发板尚未接入 Mac Studio 后，已保存 `.probe/baseline`。该快照中：

- `system_profiler SPUSBDataType` 退出码为 0，但在 macOS 26.6.2 上返回空结果。
- `ioreg -p IOUSB -l -w 0` 正常返回 USB 树。
- `/dev/cu.*` 与 `/dev/tty.*` 只出现 Bluetooth、Bose QC Earbuds 和系统 debug 节点。
- 未观察到名称包含 ESP、CP210、CH34、FTDI、USB Serial/JTAG 或 CDC 的设备/串口。

这构成开发板未连接时的正式基线，但仍不能推导目标板采用哪种 USB 实现。原始 `ioreg` 包含不断变化的统计计数器；探测脚本同时生成只保留设备树、VID/PID、产品名、厂商名、序列号和 location ID 的归一化摘要，后续差异以摘要为主、原始输出为证据补充。

## USB 连接后对比

2026-09-14，用户把开发板通过计划采用的 USB-C 线接到 Mac Studio 后，已保存 `.probe/connected` 并与基线比较。连接期间同时出现一台 iPhone；它不符合 ESP/串口筛选条件，已从目标设备判断中排除。

| 项目 | 实测结果 |
| --- | --- |
| USB product | `USB JTAG/serial debug unit` |
| Manufacturer | Espressif |
| VID:PID | `303A:1001`（十进制 `12346:4097`） |
| USB serial | `98:88:E0:06:8B:CC` |
| Location ID | `138412032` |
| Runtime callout device | `/dev/cu.usbmodem8401` |
| Runtime tty device | `/dev/tty.usbmodem8401` |
| USB implementation | ESP32-S3 原生 USB Serial/JTAG；不是 CH340、CP210 或 FTDI bridge |

`system_profiler SPUSBDataType` 在本机仍以退出码 0 返回空内容，因此本次结论由 `ioreg` 插拔差异和新增 serial device node 共同支撑。

## 运行日志与 ROM 下载链路

以 115200 baud 打开 `/dev/cu.usbmodem8401` 后，设备因原生 USB 串口打开动作发生 `USB_UART_CHIP_RESET`。这不是完全被动的读取，但没有擦除或写入 Flash。启动日志确认：

- ESP32-S3 ROM 标识 `esp32s3-20210327`，芯片 revision v0.2。
- 16 MB QIO Flash，80 MHz。
- AP 64 Mbit（8 MB）Octal PSRAM，80 MHz。
- 当前应用项目 `xiaozhi`，版本 1.9.4，ESP-IDF v5.5。
- 当前固件板型标识 `atk-dnesp32s3-box`。
- 当前固件成功初始化 LCD/LVGL、ES8311 codec 和 Wi-Fi；这些日志只证明现有固件可驱动实机，不自动证明 AgentBeacon 可复用其全部板级参数。

随后执行只读 `esptool flash_id` 探测：

- 成功连接 ESP32-S3 QFN56 revision v0.2。
- 确认 USB mode 为 `USB-Serial/JTAG`，40 MHz crystal，8 MB embedded PSRAM。
- Flash manufacturer/device 为 `68:4018`，探测容量 16 MB，3.3 V。
- RAM stub 上传、460800 baud 切换和 hard reset 均成功。

该探测证明 ROM 下载通路工作，但未执行 erase、write 或 AgentBeacon 固件烧录。实际写入后的重新枚举和运行时通信仍属于 Stage 1 实机验收。

## 当前固件源码旁证

启动日志中的 `xiaozhi` 1.9.4 与板型标识可对应到该固件的固定源码版本。其 `atk-dnesp32s3-box` board 目录记录了 ST7789 i80、XL9555 和 ES8311 等配置，可用于下一步核对实物，但它是当前运行固件的上游源码，不是正点原子的官方原理图/BSP，不能单独作为最终 GPIO 依据。

- [`xiaozhi-esp32` v1.9.4 固定提交](https://github.com/78/xiaozhi-esp32/tree/3ced7709c65a39494f5684e99111854a5bcbd8c7)
- [该版本的 `atk-dnesp32s3-box/config.h`](https://github.com/78/xiaozhi-esp32/blob/3ced7709c65a39494f5684e99111854a5bcbd8c7/main/boards/atk-dnesp32s3-box/config.h)

## Stage 0 剩余门禁

- [x] 经用户确认的未连接 USB 基线。
- [x] 连接后快照及拔插前后 USB 差异。
- [x] VID/PID 与设备名称。
- [x] native USB 或 USB-UART bridge 的证据。
- [x] ROM 下载路径与当前固件运行时串口路径；实际 AgentBeacon 烧录后仍需复验。
- [ ] PCB 完整型号/版本。
- [x] 已整理 DNESP32S3 开发板和 BOX3 两套候选官方资料，并明确不可混用。
- [ ] 确定实机版本，并取得与该版本匹配的官方 schematic、BSP 和 examples。
- [ ] LCD、touch、audio、microphone、buzzer、K0/K1/K2 的官方引脚/器件依据。
