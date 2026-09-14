# AgentBeacon 架构

## 目标与边界

AgentBeacon 把本地程序的状态事件变成实体宠物的画面与声音。ESP32 只负责确定性渲染、播报和设备 I/O；任务聚合、优先级与客户端适配由 Mac 端负责。

## 组件

### `beacond`

- 通过 `POST /v1/events` 接收通用事件，通过 `POST /v1/codex-hooks` 接收经过最小化的 Codex 生命周期事件。
- 校验并编码 Beacon Protocol 消息。
- 管理设备发现、串口连接、断线重连和设备状态。
- 把设备上报的按钮事件路由回本机消费者。
- 聚合多个 Codex 会话，生成最多 3 张“最新在最上”的任务卡；需要用户确认的会话优先控制宠物全局状态。
- 定时清除长时间没有事件的活动，避免异常退出的会话永久占用任务卡。

### `beacon`

- 作为 `beacond` 的客户端提供命令行接口。
- 不发现、打开或独占物理串口。
- 在 Stage 6 实现；Stage 2 先用 HTTP 请求验收完整链路。

### `agent-beacon-fw`

- 接收并解析 Beacon Protocol 消息。
- 驱动 LCD 和扬声器；小灯灵动画完全在盒子上运行，不依赖 Mac 端逐帧传图。
- 不解释任务生命周期之外的本机业务语义。

### `protocol`

- 定义 Beacon Protocol 的消息模型、编解码、版本和限制。
- 与 USB UART、USB CDC、WebSocket、TCP 或 Bluetooth 等 transport 无关。

## 数据流

```text
Codex Hook -> privacy filter -> POST /v1/codex-hooks --+
Other producer -----------> POST /v1/events -----------+-> beacond -> USB -> ESP32-S3
```

首版只实现 `SerialTransport`。Transport 接口只抽象连接、发送、接收和关闭所需的最小能力；在第二个真实 transport 出现前不设计复杂插件系统。

## 关键决定

1. 第一阶段采用 monorepo，Mac 端优先 Rust，固件优先评估 ESP-IDF。
2. CLI 必须经由 daemon，避免多个进程竞争串口并分散重连逻辑。
3. Beacon Protocol v1 使用 NDJSON，一行一个 JSON object。
4. 固件硬件配置必须来自精确板型的官方资料或实机验证，不借用相似板卡 GPIO。
5. 编译、模拟链路和实机验收是三种不同证据；只有实机链路满足对应阶段门禁。
6. Stage 2 的 `SerialTransport` 使用 64 条有界队列；HTTP `202 Accepted` 只表示事件已入队，不谎称设备已经处理。串口写入失败时保留当前帧，重新按 `303A:1001` 发现设备并重试。
7. 不抓取 Codex UI，也不解析不稳定的 transcript。V1 只使用官方 Hook；适配脚本在进程边界前丢弃 prompt、transcript 与工具内容。
8. 多任务卡按最近活动排序，最多 3 张；全局表情的优先级是“需要确认 > 工作中”。动画和语音资产留在固件中，Mac 只发送状态快照。
9. Hook 生命周期不保证收尾事件，活动必须能自行过期。工作中按 30 分钟、需要确认按 4 小时计时；两档时限不同，因为前者无事件通常意味着进程已经消失，后者只意味着用户尚未回来。清理由 60 秒一次的后台扫描驱动，不能只在收到新 Hook 时惰性触发。
