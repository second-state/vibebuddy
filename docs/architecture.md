# Vibe Buddy 架构

## 目标与边界

Vibe Buddy 把本地程序的状态事件变成实体宠物的画面与声音。ESP32 只负责确定性渲染、播报和设备 I/O；任务聚合、优先级与客户端适配由 Mac 端负责。

## 组件

### `vibebuddyd`

- 通过 `POST /v1/events` 接收通用事件，通过 `POST /v1/codex-hooks` 和 `POST /v1/claude-hooks` 接收经过最小化的 Agent 生命周期事件。
- 校验并编码 Vibe Buddy Protocol 消息。
- 管理设备发现、串口连接、断线重连和设备状态。
- 把设备上报的按钮事件路由回本机消费者。
- 聚合所有 Agent 的会话，生成最多 3 张“最新在最上”的任务卡；需要用户确认的活动优先控制宠物全局状态。
- 定时清除长时间没有事件的活动，避免异常退出的会话永久占用任务卡。

### `beacon`

- 作为 `vibebuddyd` 的客户端提供命令行接口。
- 不发现、打开或独占物理串口。
- 在 Stage 6 实现；Stage 2 先用 HTTP 请求验收完整链路。

### `vibebuddy-fw`

- 接收并解析 Vibe Buddy Protocol 消息。
- 驱动 LCD 和扬声器；氛围小助手动画完全在盒子上运行，不依赖 Mac 端逐帧传图。
- 不解释任务生命周期之外的本机业务语义。

### `app`（设计已定，待实现）

- 菜单栏 App，看管 `vibebuddyd`（作为 helper 随 App 分发、启动、重启、退出），取代 LaunchAgent。
- 首次引导、接入 Agent（写 Hook 配置）、挑选播报音色并写入设备、更新固件、看设备画面。
- 只在链路异常时自己出声（macOS 通知）；Agent 的事仍由设备讲。设计见 [`app.md`](app.md)。

### `protocol`

- 定义 Vibe Buddy Protocol 的消息模型、编解码、版本和限制。
- 与 USB UART、USB CDC、WebSocket、TCP 或 Bluetooth 等 transport 无关。

## 数据流

```text
Codex Hook -------> privacy filter -> POST /v1/codex-hooks ---+
Claude Code Hook -> privacy filter -> POST /v1/claude-hooks --+
Other producer ------------------> POST /v1/events -----------+-> vibebuddyd -> USB -> ESP32-S3
Codex / Claude / Browser <---------- open source <------------+<- vibebuddyd <- K2
```

首版只实现 `SerialTransport`。Transport 接口只抽象连接、发送、接收和关闭所需的最小能力；在第二个真实 transport 出现前不设计复杂插件系统。

## 关键决定

1. 第一阶段采用 monorepo，Mac 端优先 Rust，固件优先评估 ESP-IDF。
2. CLI 必须经由 daemon，避免多个进程竞争串口并分散重连逻辑。
3. Vibe Buddy Protocol v1 使用 NDJSON，一行一个 JSON object。
4. 固件硬件配置必须来自精确板型的官方资料或实机验证，不借用相似板卡 GPIO。
5. 编译、模拟链路和实机验收是三种不同证据；只有实机链路满足对应阶段门禁。
6. Stage 2 的 `SerialTransport` 使用 64 条有界队列；HTTP `202 Accepted` 只表示事件已入队，不谎称设备已经处理。串口写入失败时保留当前帧，重新按 `303A:1001` 发现设备并重试。
7. 不抓取 Codex UI，也不解析不稳定的 transcript。V1 只使用官方 Hook；适配脚本在进程边界前丢弃 prompt、transcript 与工具内容。任务卡的会话名来自两个 App 自己存在本机的记录（Claude App 会话索引的 `title`、Codex `state_5.sqlite` 里 `threads` 的 `name` 与 `git_branch`），它们是 App 侧栏里本来就显示的名字，不是 prompt；Codex 的 `title` 列是原始首条消息，不读。查询有缓存，Hook 每几秒来一次，不能每次都扫目录、开数据库。
8. 多任务卡按最近活动排序，最多 3 张；全局表情的优先级是“需要确认 > 工作中”。动画和语音资产留在固件中，Mac 只发送状态快照。
9. Hook 生命周期不保证收尾事件，活动必须能自行过期。工作中按 30 分钟、需要确认按 4 小时计时；两档时限不同，因为前者无事件通常意味着进程已经消失，后者只意味着用户尚未回来。清理由 60 秒一次的后台扫描驱动，不能只在收到新 Hook 时惰性触发。
10. 每个 Agent 一个 Adapter，但所有 Adapter 写入同一个聚合器。设备只有一块屏幕和一只氛围小助手，两个聚合器会各自维护任务卡栈并互相覆盖。Adapter 因此不持有状态，只做事件翻译和活动身份合成；任务卡标题带 Agent 前缀，因为两个 Agent 常在同一目录下工作。
11. CI 与 Agent 共用同一套任务卡和播报，只是来源不同：GitHub Actions 由 `vibebuddyd` 主动轮询，不靠 Hook。只有亲眼见过某次 run 在跑，它结束时才播报；否则每次重启 daemon 都会把仓库里最近一次历史结果重新宣告一遍。关注哪些仓库不需要用户配置：Hook 已经带来了 `cwd`，Agent 最近工作过的项目就是该关心 CI 的那些，仓库名从本地 `.git/config` 推导。
12. K2 打开的来源必须与屏幕主状态使用同一套优先级，不能另做一个“最近窗口”列表。聚合器只保存定位所需的 thread id、session id 或 repo/run id，不保存 prompt 与回复；外部参数不经过 shell。Codex 子 Agent 的生命周期 session 不是桌面可导航 thread，Hook 必须把它映射到父 thread 后再关联来源；K2 的落点由运行处决定：跑在 Agent 自己的桌面应用里才用 deeplink，跑在别的应用里就把那个应用拉到前台，没有宿主的会话跳过。运行处与桌面会话 id 都由 Hook 从进程环境判定后上报，daemon 只做路由，不去磁盘上猜——按 CLI `session_id` 认领 transcript 是一对多的，会打开影子会话。
13. 番茄钟、休闲与模式切换都在固件里，`vibebuddyd` 只收诊断行。它是设备 I/O 与确定性渲染，本来就是固件的职责；更要紧的是用户拿它计时时 Mac 端可能没在跑，一个要靠心跳才走的计时器没有用。固件里凡是不碰硬件的逻辑——番茄钟状态机、休闲导演、绘制代码——都要能在 Mac 上直接编译：状态机与导演有主机测试，画面与剧目有主机预览，烧录之前就能看见版式和动画。
14. Mac 端的图形界面是原生 SwiftUI 菜单栏 App，`vibebuddyd` 作为 helper 由它看管；语音包放独立分区、走串口协议写入；Hook 改为 Rust 单二进制复制到 Application Support。三条取舍分别见 ADR-0004、0003、0005，整体设计见 [`app.md`](app.md)。
