# Stage 1 — Serial Hello 验收记录

验收时间：2026-09-14（Asia/Singapore）

## 结论

Stage 1 **已通过实机验收**。证据链是 Mac Studio → `/dev/cu.usbmodem8401` → ESP32-S3 原生 USB Serial/JTAG → `agent-beacon-fw` → cJSON parse → USB 返回结果，不是本机模拟或仅编译通过。

本阶段没有初始化 LCD、audio、microphone、buzzer 或 buttons。实机烧录后，LCD 继续显示原小智固件最后留下的配网页面；这不是旧固件仍在运行，而是 LCD 控制器显存和背光在 ESP32 软件复位后保持，且 Stage 1 固件没有覆盖画面。随后重复执行 Serial Hello 仍通过，直接证明当前运行的是 AgentBeacon 固件。LCD 清屏和新 UI 必须等待 Stage 3 及厂家硬件依据。

## 构建

- ESP-IDF：v5.5.3。
- Target：`esp32s3`。
- 固件项目名：`agent-beacon-fw`。
- 应用镜像：183,616 bytes（`0x2cd40`）。
- 默认应用分区：1 MiB，剩余 82%。
- 首次构建发现并移除了一个无效 Kconfig symbol 和一个 C qualifier warning；重新构建无编译警告。

构建和烧录入口：

```bash
./tools/flash.sh /dev/cu.usbmodem8401
```

## 原厂固件备份

写入 AgentBeacon 前，使用 `esptool read_flash` 读取了完整 16 MiB Flash：

- 本机文件：`.probe/factory/atk-dnesp32s3-box-v1.1-xiaozhi-1.9.4-2026-09-14.bin`
- 大小：16,777,216 bytes。
- SHA-256：`7e0ae33002423eaca46a6e6c1f8cc6d92a2a04babd99fe7aa2a4ede788a01ec2`。
- 权限：`600`。
- Git 状态：`.probe/` 已忽略，不进入仓库。

整片备份可能包含原固件的配网或设备状态，因此按敏感本机证据处理，不复制到文档或远程仓库。

## 烧录

`idf.py flash` 通过同一 `/dev/cu.usbmodem8401` 成功写入：

| Offset | 内容 | 写入大小 |
| --- | --- | --- |
| `0x0000` | bootloader | 20,832 bytes |
| `0x8000` | partition table | 3,072 bytes |
| `0x10000` | `agent-beacon-fw` | 183,616 bytes |

三段写入均由 esptool 报告 hash verified，随后完成 hard reset；设备仍以 `/dev/cu.usbmodem8401` 重新枚举。

## Mac → USB → ESP32 验收

执行：

```bash
uv run --with pyserial python tools/serial-hello.py /dev/cu.usbmodem8401
```

Mac 实际发送：

```json
{"version":1,"event":"task.done","title":"Hello"}
```

ESP32 实际返回：

```text
EVENT task.done
TITLE Hello
PASS Stage 1 Mac -> USB -> ESP32-S3 -> JSON parse
```

在用户观察到旧小智画面仍停留后，同一命令再次得到完全相同的 PASS，排除了“烧录未生效或仍在运行旧固件”。Hello 没有 `id`，验证了 Stage 1 对可选 `id` 的实际兼容。NDJSON framing、版本策略、未知字段/事件和输入上限见 [`protocol.md`](protocol.md)。

同一实机还补测了非法 JSON、`version: 2`、带未知字段的 `custom.event`、CRLF framing 和 1025-byte 超限输入，依次得到：

```text
ERROR invalid_json
ERROR unsupported_version
EVENT custom.event
ERROR input_too_large
PASS protocol error, version, extension, CRLF, and size handling
```

## 未证明的事项

- 这次验收没有证明 LCD、audio、buzzer、buttons 或 TF 卡可由 AgentBeacon 驱动。
- 这次验收没有实现 `beacond`、HTTP API 或 serial reconnect；这些属于 Stage 2。
- 当前串口节点编号可能随 macOS 枚举变化，后续 daemon 不能写死 `usbmodem8401`，应使用 VID/PID 与 USB serial 发现设备。
