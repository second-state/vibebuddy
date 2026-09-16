Vibe Buddy (formerly AgentBeacon) is a physical status terminal for local AI agents and long-running tasks.

# Vibe Buddy

Vibe Buddy（原名 AgentBeacon，仓库、`beacond` 与 Beacon Protocol 沿用旧名）是运行在 ESP32-S3 盒子里的实体 Agent 宠物。原创角色“氛围小助手”用动画、任务卡片和短语音呈现本地 AI Agent 与长任务的状态；Codex 是第一个适配器，但设备协议不绑定某个客户端。

## 当前状态

**Stage 4 — 氛围小助手显示与语音已通过实机验收，Codex、Claude Code 与 GitHub Actions 均已接入。** 它们的事件都经 `beacond` 和 USB Serial/JTAG 到达盒子，并共享同一个任务卡栈。氛围小助手支持空闲、工作中、需要确认、完成、失败和失联；最多显示 3 张任务卡，最新在最上，每张卡显示它在当前状态里待了多久；需要确认、完成和失败各播报一次短语音，工作中保持安静；空闲时轮播当日战绩并偶尔做个小动作。

氛围小助手有三个模式。**值班**是默认：盯着 Agent，有事叫你。**番茄钟**给你计时（专注 25 分钟、休息 5 分钟）：K0 开始、暂停、继续，长按放弃；K1 在值班与番茄钟之间切换；K2 照旧打开来源，长按静音；阶段结束响铃并播报，下一阶段等你按 K0 再开始；今天完成了几次、专注了多久记在面板上，按日清零，重启不丢。**休闲**是值班空闲够久之后它自己去玩：五分钟后开始演小剧目（巡逻、踢球、看书、数星星、躲猫猫、被自己吓醒、梦话），半小时后困了转暗睡觉，夜里睡够 90 分钟关背光；Agent 一有动静或按任何键立刻回来值班。番茄钟与休闲的状态都在固件里，不依赖 Mac 端。设计见 [`docs/pomodoro.md`](docs/pomodoro.md) 与 [`docs/leisure.md`](docs/leisure.md)。

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

更完整的边界与决定见 [`docs/architecture.md`](docs/architecture.md)，领域词汇见 [`CONTEXT.md`](CONTEXT.md)，氛围小助手设计见 [`docs/pet.md`](docs/pet.md)，番茄钟见 [`docs/pomodoro.md`](docs/pomodoro.md)，休闲模式见 [`docs/leisure.md`](docs/leisure.md)，Codex 接入见 [`docs/codex-adapter.md`](docs/codex-adapter.md)，CI 接入见 [`docs/ci.md`](docs/ci.md)，协议决定见 [`docs/protocol.md`](docs/protocol.md)，阶段门禁见 [`docs/roadmap.md`](docs/roadmap.md)，外部项目的借鉴边界见 [`docs/references.md`](docs/references.md)。

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

烧录前可以在 Mac 上先验证固件里不依赖硬件的部分：`./tools/test-pomodoro.sh` 跑番茄钟状态机的测试，`./tools/preview-display.sh` 把固件的绘制代码渲染成 PNG 看版式。

固件使用 [`firmware/partitions.csv`](firmware/partitions.csv) 的自定义分区表（app 分区 4 MB），因为语音资产已经装不进默认的 1 MB。这个选择写在 `sdkconfig.defaults` 里，但 ESP-IDF 只在生成 `sdkconfig` 时读取 defaults：2026-09-15 之前就存在 `firmware/sdkconfig` 的检出目录要先删掉它再构建，否则仍按旧分区表检查大小并失败。

设备若只接着 BOX 的 `UART` 口（CH343 桥，`/dev/cu.usbmodem5909…`），不要用 `flash.sh`：那条路在 esptool 默认参数下会把 flash 擦掉后写不进去。用 [`tools/flash-bridge.sh`](tools/flash-bridge.sh)，它以 `--no-stub` 加 256 字节写块烧录，先单独写分区表试路，再写 app：

```bash
launchctl bootout gui/$(id -u)/com.agentbeacon.beacond
./tools/flash-bridge.sh /dev/cu.usbmodem59090668961 partition
./tools/flash-bridge.sh /dev/cu.usbmodem59090668961 app
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.agentbeacon.beacond.plist
```

`flash.sh` 会覆盖当前固件。2026-09-14 的原厂 `xiaozhi` 1.9.4 整片备份保存在本机 `.probe/factory/`，权限为 `600`，不会提交到 Git。

## Vibe Buddy App

Mac 端的图形界面是一个菜单栏 App，它把 `beacond` 与 `beacon-hook` 带在身上并看管 daemon，取代了 LaunchAgent。装包：

```bash
idf.py -C firmware build          # Release 装包要附带固件三件套；--debug 可以不带
app/scripts/build-app.sh          # 出 app/build/Vibe Buddy.app
open "app/build/Vibe Buddy.app"
```

首次启动走引导：找盒子（眨眼确认）、接入 Codex 与 Claude Code（写 Hook 配置前展示差异）、挑播报音色写进盒子、登录时启动。之后菜单栏图标回答"盒子在线吗、什么模式、今天干了多少"，设置窗五页管通用、声音、接入、设备（固件更新、截图）、高级。发现旧的 LaunchAgent 会提议卸掉并接管。设计见 [`docs/app.md`](docs/app.md)。

App 用 SwiftPM 构建，只需要命令行工具；`swift run --package-path app SelfTest` 跑视图模型的自检。

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

daemon 由 Vibe Buddy App 看管（见上）。没有 App 的开发机可以用 [`packaging/com.agentbeacon.beacond.plist`](packaging/com.agentbeacon.beacond.plist) 装成 LaunchAgent，但两者不能同时跑，会抢串口。App 还提供 `GET /v1/status`、SSE `/v1/status/stream`、`/v1/config`、`/v1/device/{identify,screenshot,voice-pack,firmware}` 与 `/v1/daemon/restart`。

经 BOX 的 CH343 UART 桥发送时 daemon 按线速分段写：这条桥一次吞不下超过两百字节的连续数据，会把内容错位而长度不变；原生 USB 口不受影响。教训见 [`LESSONS.md`](LESSONS.md)。

## Agent 接入

Codex 与 Claude Code 都由本机 Hook 接入，各有一个隐私过滤脚本，两者写入同一个聚合器。任务卡标题带 Agent 前缀：Codex 为 `CX:`，Claude Code 为 `CC:`。第一行写 Agent 自己给会话起的名字（Claude App 的会话标题、Codex 的线程名或分支），第二行写项目名；会话没有名字时第一行就是项目名。项目名取自 git 项目根，因此在子目录或 worktree 中工作时显示的仍是项目名。

Hook 是一个 Rust 二进制 [`hook/`](hook/)（`beacon-hook codex` / `beacon-hook claude`），App 把它复制到 `~/Library/Application Support/AgentBeacon/bin/` 并写进用户级配置；不依赖 python3，App 挪位置也不断（ADR-0005）。

- Codex：`~/.codex/hooks.json` 的六个事件，写入后要在 Codex 的 `/hooks` 页面审查、信任；详见 [`docs/codex-adapter.md`](docs/codex-adapter.md)。
- Claude Code：`~/.claude/settings.json` 的八个事件；详见 [`docs/claude-adapter.md`](docs/claude-adapter.md)。

它只抽取会话与回合标识、事件名和工作目录，不转发 prompt、助手回复、transcript 或工具结果；判断助手是否在等待回答的规则两个 Agent 共用。`tools/codex-hook.py`、`tools/claude-hook.py` 是它的前身，配置还指着它们的机器在 App 的接入页点「修复」即可换过来，换完这两个脚本就可以删了。

## CI 接入

GitHub Actions 不走 Hook，由 `beacond` 每 30 秒用 `gh run list` 主动轮询，沿用你已有的 GitHub 登录。**不需要配置**：关注哪些仓库由 Agent 最近一小时工作过的项目自动推导，`owner/repo` 从 `.git/config` 的 `origin` 远端读出。任务卡前缀为 `CI:`，详见 [`docs/ci.md`](docs/ci.md)。

## 仓库布局

```text
daemon/    # beacond；Stage 2 开始实现
cli/       # beacon；Stage 6 开始实现
protocol/  # Beacon Protocol 类型与编解码
firmware/  # agent-beacon-fw
docs/      # 架构、协议、硬件证据和路线图
tools/     # 探测与烧录辅助脚本
```
