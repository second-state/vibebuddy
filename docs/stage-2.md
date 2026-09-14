# Stage 2 — `beacond` 验收记录

验收时间：2026-09-14（Asia/Singapore）

## 结论

Stage 2 **已通过实机验收**。Mac 本机 HTTP 请求经 `beacond`、`SerialTransport`、ESP32-S3 原生 USB Serial/JTAG 和 Beacon Protocol 到达 `agent-beacon-fw`；真实 USB 拔插后 daemon 自动重新发现并连接设备，重连后的事件也成功送达。

## 实现边界

- `POST /v1/events` 默认只监听 `127.0.0.1:7331`。
- `beacon-protocol` 负责 version、非空 event、未知扩展字段、NDJSON 编码和 1024-byte 上限。
- `SerialTransport` 默认按 `VID:PID 303A:1001` 发现唯一设备，不写死 `/dev/cu.usbmodem8401`。
- `BEACON_SERIAL_PORT` 可显式指定串口；`BEACON_USB_SERIAL` 可在多块同型号设备中筛选目标。
- 发送队列容量为 64。HTTP `202 Accepted` 只表示入队；队列满或 worker 已停止时返回 `503 Service Unavailable`。
- 串口读写失败后保留尚未确认 flush 的当前帧，每 500 ms 重新发现并连接设备。

本阶段没有安装 macOS 后台服务，也没有实现 `beacon` CLI、LCD、audio 或 buttons。

## 自动化检查

执行：

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

结果：6 个测试通过，Clippy 无告警。测试覆盖无 `id` 的 Hello、未知字段保留、不支持的 version、超长消息、HTTP 入队与 NDJSON framing，以及 USB serial 格式归一化。

## HTTP → USB → ESP32 实机链路

daemon 自动发现并打开：

```text
串口已连接 port=/dev/cu.usbmodem8401
```

发送：

```json
{"version":1,"event":"task.start","id":"stage2-live","title":"Stage 2 HTTP"}
```

HTTP 实际返回 `202 Accepted`。ESP32 随后实际返回：

```text
EVENT task.start
TITLE Stage 2 HTTP
```

`version: 2` 的请求实际返回 `400 Bad Request`，没有进入串口队列。

## 真实断线重连

保持同一 `beacond` 进程运行，用户拔掉 `USB-SLAVE` 后日志记录：

```text
串口读取失败，开始重连 port=/dev/cu.usbmodem8401 error=Device not configured (os error 6)
```

重新插回后，无需重启 daemon：

```text
串口已连接 port=/dev/cu.usbmodem8401
```

随后发送：

```json
{"version":1,"event":"task.done","id":"stage2-reconnect","title":"Reconnect verified"}
```

设备实际返回：

```text
EVENT task.done
TITLE Reconnect verified
```

因此本阶段证明的是同一 daemon 进程在真实 USB 断开、重新枚举后恢复传输，不是进程重启或本地模拟。
