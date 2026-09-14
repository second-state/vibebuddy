AgentBeacon is a physical status terminal for local AI agents and long-running tasks.

# AgentBeacon

AgentBeacon 是面向本地 AI Agent、开发工具和长时间运行任务的桌面硬件状态终端，不绑定某个 ChatGPT/Codex 客户端。未来可接入本地 Agent、Python/Rust、Shell、视频生成、模型训练、FFmpeg/Blender、CI/Build 等。

## 当前状态

**Stage 2 — `beacond` 已通过实机验收。** 本机 HTTP 请求已经经由 `beacond`、自动发现的 ESP32-S3 原生 USB Serial/JTAG 和 Beacon Protocol 到达固件；真实拔插后 daemon 自动重新发现并连接设备，重连后的事件也已送达。设备当前不初始化 LCD、audio 或 buttons；Stage 3 正在调查准确屏幕参数和 BSP。

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

更完整的边界与决定见 [`docs/architecture.md`](docs/architecture.md)，协议决定见 [`docs/protocol.md`](docs/protocol.md)，阶段门禁见 [`docs/roadmap.md`](docs/roadmap.md)，实机证据见 [`docs/stage-0.md`](docs/stage-0.md)、[`docs/stage-1.md`](docs/stage-1.md) 和 [`docs/stage-2.md`](docs/stage-2.md)，外部项目的借鉴边界见 [`docs/references.md`](docs/references.md)。

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

## Stage 2 daemon

启动 `beacond`：

```bash
cargo run -p beacond
```

默认只监听 `127.0.0.1:7331`，并按 Espressif USB Serial/JTAG 的 `VID:PID 303A:1001` 自动发现设备。发送事件：

```bash
curl -H 'content-type: application/json' \
  --data '{"version":1,"event":"task.done","title":"Hello"}' \
  http://127.0.0.1:7331/v1/events
```

可用 `BEACON_BIND` 修改监听地址、`BEACON_SERIAL_PORT` 显式指定串口，或用 `BEACON_USB_SERIAL` 在多块相同设备中选择目标。HTTP `202 Accepted` 表示事件进入有界发送队列；设备实际接收结果以 daemon 记录的设备响应为准。

## 仓库布局

```text
daemon/    # beacond；Stage 2 开始实现
cli/       # beacon；Stage 6 开始实现
protocol/  # Beacon Protocol 类型与编解码
firmware/  # agent-beacon-fw
docs/      # 架构、协议、硬件证据和路线图
tools/     # 探测与烧录辅助脚本
```
