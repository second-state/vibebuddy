AgentBeacon is a physical status terminal for local AI agents and long-running tasks.

# AgentBeacon

AgentBeacon 是面向本地 AI Agent、开发工具和长时间运行任务的桌面硬件状态终端，不绑定某个 ChatGPT/Codex 客户端。未来可接入本地 Agent、Python/Rust、Shell、视频生成、模型训练、FFmpeg/Blender、CI/Build 等。

## 当前状态

**Stage 1 — Serial Hello 已通过实机验收。** Mac Studio 已通过同一根 USB-C 线完成 ESP32-S3 原生 USB Serial/JTAG 枚举、固件烧录和运行时 NDJSON 传输。设备当前运行最小 `agent-beacon-fw`，不初始化 LCD、audio 或 buttons；Stage 2 尚未开始。

## 目标架构

```text
Local Programs / Agents / Codex / Scripts
                |
                | HTTP / Unix Socket
                v
             beacond
                |
                | Transport abstraction
                v
          USB Serial / USB CDC
                |
                v
           ESP32S3-BOX
                |
      LCD / Speaker / Buzzer / Buttons
```

- `beacond`：本机守护进程，负责事件接入、设备连接、重连、路由和状态。
- `beacon`：只调用 `beacond` 的命令行客户端，不直接占用串口。
- `agent-beacon-fw`：ESP32-S3 固件，仅负责设备 I/O 和 Beacon Protocol 消息处理。
- Beacon Protocol：与 transport 解耦的可扩展 NDJSON 协议。

更完整的边界与决定见 [`docs/architecture.md`](docs/architecture.md)，协议决定见 [`docs/protocol.md`](docs/protocol.md)，阶段门禁见 [`docs/roadmap.md`](docs/roadmap.md)，实机证据见 [`docs/stage-0.md`](docs/stage-0.md) 和 [`docs/stage-1.md`](docs/stage-1.md)，外部项目的借鉴边界见 [`docs/references.md`](docs/references.md)。

## Stage 0 USB 探测

在开发板未连接和已连接两种状态下分别运行：

```bash
./tools/detect-device.sh baseline
./tools/detect-device.sh connected
diff -ru .probe/baseline .probe/connected
```

探测结果可能包含本机 USB 设备标识。`.probe/` 默认不纳入 Git。

## Stage 1 固件

连接 ATK-DNESP32S3-BOX V1.1 的 `USB-SLAVE` 口后执行：

```bash
./tools/flash.sh /dev/cu.usbmodem8401
uv run --with pyserial python tools/serial-hello.py /dev/cu.usbmodem8401
```

`flash.sh` 会覆盖当前固件。2026-09-14 的原厂 `xiaozhi` 1.9.4 整片备份保存在本机 `.probe/factory/`，权限为 `600`，不会提交到 Git。

## 仓库布局

```text
daemon/    # beacond；Stage 2 开始实现
cli/       # beacon；Stage 6 开始实现
protocol/  # Beacon Protocol 类型与编解码
firmware/  # agent-beacon-fw
docs/      # 架构、协议、硬件证据和路线图
tools/     # 探测与烧录辅助脚本
```
