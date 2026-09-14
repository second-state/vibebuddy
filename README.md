AgentBeacon is a physical status terminal for local AI agents and long-running tasks.

# AgentBeacon

AgentBeacon 是运行在 ESP32-S3 盒子里的实体 Agent 宠物。原创角色“小灯灵”用动画、任务卡片和短语音呈现本地 AI Agent 与长任务的状态；Codex 是第一个适配器，但设备协议不绑定某个客户端。

## 当前状态

**Stage 4 — 小灯灵显示与语音已通过实机验收。** Codex 生命周期事件可经本机 Hook、`beacond`、USB Serial/JTAG 到达盒子。小灯灵支持空闲、工作中、需要确认、完成和失败动画；最多显示 3 张任务卡，最新在最上；需要确认、完成和失败各播报一次短语音，工作中保持安静。

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
      LCD / Speaker / Buttons
```

- `beacond`：本机守护进程，负责事件接入、设备连接、重连、路由和状态。
- `beacon`：只调用 `beacond` 的命令行客户端，不直接占用串口。
- `agent-beacon-fw`：ESP32-S3 固件，仅负责设备 I/O 和 Beacon Protocol 消息处理。
- Beacon Protocol：与 transport 解耦的可扩展 NDJSON 协议。

更完整的边界与决定见 [`docs/architecture.md`](docs/architecture.md)，小灯灵设计见 [`docs/pet.md`](docs/pet.md)，Codex 接入见 [`docs/codex-adapter.md`](docs/codex-adapter.md)，协议决定见 [`docs/protocol.md`](docs/protocol.md)，阶段门禁见 [`docs/roadmap.md`](docs/roadmap.md)，外部项目的借鉴边界见 [`docs/references.md`](docs/references.md)。

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

当前 Mac Studio 已安装 `com.agentbeacon.beacond` LaunchAgent，登录后会自动运行 release binary；配置模板在 [`packaging/com.agentbeacon.beacond.plist`](packaging/com.agentbeacon.beacond.plist)。

## Codex 宠物接入

仓库的 [`tools/codex-hook.py`](tools/codex-hook.py) 只抽取 Hook 的 `session_id`、`turn_id`、`hook_event_name` 和 `cwd`；对于等待用户回答的 `Stop`，它会在本机生成 `response_kind`。脚本不会转发 prompt、助手回复、transcript 或工具结果。用户级 `~/.codex/hooks.json` 需要为六个生命周期事件配置该脚本，并在 Codex 的 `/hooks` 页面审查、信任配置；详见 [`docs/codex-adapter.md`](docs/codex-adapter.md)。

## 仓库布局

```text
daemon/    # beacond；Stage 2 开始实现
cli/       # beacon；Stage 6 开始实现
protocol/  # Beacon Protocol 类型与编解码
firmware/  # agent-beacon-fw
docs/      # 架构、协议、硬件证据和路线图
tools/     # 探测与烧录辅助脚本
```
