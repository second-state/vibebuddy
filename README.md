AgentBeacon is a physical status terminal for local AI agents and long-running tasks.

# AgentBeacon

AgentBeacon 是面向本地 AI Agent、开发工具和长时间运行任务的桌面硬件状态终端，不绑定某个 ChatGPT/Codex 客户端。未来可接入本地 Agent、Python/Rust、Shell、视频生成、模型训练、FFmpeg/Blender、CI/Build 等。

## 当前状态

项目处于 **Stage 0 — Hardware Probe**。当前只建立架构、路线图和可重复的 Mac USB 探测流程；在准确板型、USB 枚举、烧录路径和运行时串口得到实机确认前，不进入固件实现。

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

更完整的边界与决定见 [`docs/architecture.md`](docs/architecture.md)，阶段门禁见 [`docs/roadmap.md`](docs/roadmap.md)，当前 Stage 0 证据见 [`docs/stage-0.md`](docs/stage-0.md)。

## Stage 0 USB 探测

在开发板未连接和已连接两种状态下分别运行：

```bash
./tools/detect-device.sh baseline
./tools/detect-device.sh connected
diff -ru .probe/baseline .probe/connected
```

探测结果可能包含本机 USB 设备标识。`.probe/` 默认不纳入 Git。

## 仓库布局

```text
daemon/    # beacond；Stage 2 开始实现
cli/       # beacon；Stage 6 开始实现
protocol/  # Beacon Protocol 类型与编解码
firmware/  # agent-beacon-fw
docs/      # 架构、协议、硬件证据和路线图
tools/     # 探测与烧录辅助脚本
```
