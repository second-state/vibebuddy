# Stage 0 — Hardware Probe 记录

更新时间：2026-09-13（Asia/Singapore）

## 当前结论

Stage 0 **尚未通过**。开发主机身份已经确认，通用开发环境前置依赖已经补齐；但还没有完成经用户确认的 USB 拔插对比，也没有确认准确 PCB 型号、VID/PID、烧录路径和运行时串口。

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

安装 EIM 时，Homebrew 自动从 6.0.22 更新到 7.0.0，并要求显式信任第三方 tap。仅对 Espressif 官方 `espressif/eim` tap 建立了信任；没有处理或信任其他 tap。EIM 当前报告没有已安装的 ESP-IDF 版本。

暂不执行 `eim install`：需要先从精确板型的官方 BSP/示例确认兼容的 ESP-IDF 版本。EIM 支持并存和选择不同版本，因此确定版本后再安装不会阻塞当前 USB 枚举。

官方依据：

- [ESP-IDF v6.0 macOS 安装说明](https://docs.espressif.com/projects/esp-idf/en/stable/esp32p4/get-started/macos-setup.html)
- [EIM 官方说明与 macOS Homebrew 安装方式](https://docs.espressif.com/projects/idf-im-ui/en/latest/)
- [EIM 前置依赖](https://docs.espressif.com/projects/idf-im-cli/en/latest/prerequisites.html)

## USB 初始观察

未经过用户确认连接状态的 `pre-change` 快照中：

- `system_profiler SPUSBDataType` 退出码为 0，但在 macOS 26.6.2 上返回空结果。
- `ioreg -p IOUSB -l -w 0` 正常返回 USB 树。
- `/dev/cu.*` 与 `/dev/tty.*` 只出现 Bluetooth、Bose QC Earbuds 和系统 debug 节点。
- 未观察到名称包含 ESP、CP210、CH34、FTDI、USB Serial/JTAG 或 CDC 的设备/串口。

这只能说明该时刻没有观察到目标设备，不能证明开发板未连接，也不能推导目标板采用哪种 USB 实现。

## 待执行的实机对比

1. 用户确认开发板已经拔开。
2. 运行 `./tools/detect-device.sh baseline`。
3. 用户用计划采用的 USB-C 线把开发板直接接到 Mac Studio，并报告供电/指示现象。
4. 运行 `./tools/detect-device.sh connected`。
5. 对 `.probe/baseline` 与 `.probe/connected` 做结构化差异，记录 VID/PID、产品名、串口节点和 USB 类型。
6. 必要时再让用户操作 BOOT/RESET，区分正常运行枚举与下载模式枚举。

## Stage 0 剩余门禁

- [ ] 经确认的拔插前后 USB 差异。
- [ ] VID/PID 与设备名称。
- [ ] native USB 或 USB-UART bridge 的证据。
- [ ] 烧录路径与运行时串口路径。
- [ ] PCB 完整型号/版本。
- [ ] 与该版本匹配的官方 schematic、BSP 和 examples。
- [ ] LCD、touch、audio、microphone、buzzer、K0/K1/K2 的官方引脚/器件依据。

