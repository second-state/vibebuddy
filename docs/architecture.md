# AgentBeacon 架构

## 目标与边界

AgentBeacon 把本地程序的状态事件传到桌面硬件终端。ESP32 不承载任务编排或业务规则；Mac 端也不依赖某个特定 Agent 客户端。

## 组件

### `beacond`

- 通过 HTTP（首个端点为 `POST /v1/events`）接收本机事件。
- 校验并编码 Beacon Protocol 消息。
- 管理设备发现、串口连接、断线重连和设备状态。
- 把设备上报的按钮事件路由回本机消费者。

### `beacon`

- 作为 `beacond` 的客户端提供命令行接口。
- 不发现、打开或独占物理串口。
- 在 Stage 6 实现；Stage 2 先用 HTTP 请求验收完整链路。

### `agent-beacon-fw`

- 接收并解析 Beacon Protocol 消息。
- 驱动 LCD、声音和按键，并上报设备事件。
- 不解释任务生命周期之外的本机业务语义。

### `protocol`

- 定义 Beacon Protocol 的消息模型、编解码、版本和限制。
- 与 USB UART、USB CDC、WebSocket、TCP 或 Bluetooth 等 transport 无关。

## 数据流

```text
Producer -> HTTP/Unix Socket -> beacond -> Transport -> ESP32-S3
Producer <- HTTP/Unix Socket <- beacond <- Transport <- ESP32-S3 button event
```

首版只实现 `SerialTransport`。Transport 接口只抽象连接、发送、接收和关闭所需的最小能力；在第二个真实 transport 出现前不设计复杂插件系统。

## 关键决定

1. 第一阶段采用 monorepo，Mac 端优先 Rust，固件优先评估 ESP-IDF。
2. CLI 必须经由 daemon，避免多个进程竞争串口并分散重连逻辑。
3. Beacon Protocol v1 使用 NDJSON，一行一个 JSON object。
4. 固件硬件配置必须来自精确板型的官方资料或实机验证，不借用相似板卡 GPIO。
5. 编译、模拟链路和实机验收是三种不同证据；只有实机链路满足对应阶段门禁。

