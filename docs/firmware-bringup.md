# Rust 固件第一次上机

2026-09-26 固件改写成 Rust（ADR-0006）时手边没有盒子。当时 Mac 上能验证的都验证了，剩下只有实机才能回答的问题，按下面的顺序过一遍。

## 已经在 Mac 上验证过的

- `just test-firmware`：firmware-core 的 51 个测试，外加 Rust 与 C 两份固件的画面逐像素比对（826 帧，含每个休闲剧目的每一帧）。
- 整条串口链：假板子上的端到端测试。另外还有模拟器（`tools/simulate-device.py`），可以对着它跑 `tools/firmware-smoke.py`，16 项全过。
- ES8311 寄存器序列：对照 esp_codec_dev 1.6.2 逐条列出，写进测试。
- 分区表：与 ESP-IDF 的 `gen_esp32part.py` 输出逐字节一致。
- 构建标识：源码不改、隔一分钟再构建，标识里的时刻跟着变。

## 只有实机能回答的

| 风险 | 表现 | 先看哪里 |
|---|---|---|
| ST7789 初始化或 i8080 时序 | 串口报 `DISPLAY ERROR`，或背光亮但花屏、黑屏 | `firmware-rs/core/src/lcd.rs` 的命令表；`device/src/lcd.rs` 里无参数命令（SWRESET、SLPOUT、INVON、DISPON）带一个哑参数字节发——esp-hal 的 i8080 没法像 ESP-IDF 那样关掉数据阶段，这是唯一与 C 固件不同的地方 |
| 颜色或方向不对 | 红蓝对调、整屏镜像、上下颠倒 | MADCTL（0x36）的取值；像素字节序（帧缓冲按大端存） |
| ES8311 或 I2S 格式 | `AUDIO QUEUED` 有，没声音；或者声音变调、有杂音 | `core/src/audio.rs` 的序列；`main.rs` 的 `TdmConfig` |
| 音频流断了没恢复 | 写完语音包之后再也不出声 | `device/src/audio.rs` 的断流检测 |
| UART 接收 | 只接 UART 桥时 `invalid_json` 变多 | `device/src/serial.rs` 的中断与环形缓冲 |
| USB 输出卡住 | 只接 UART 桥、过夜后画面定格（见 LESSONS.md「没有对端的输出通道」） | `serial.rs` 的 `write_usb` |
| 设置存储 | 重启后番茄记录或音量不对 | `core/src/storage.rs` |

## 与 C 固件有意不同的地方

- 设置（当日番茄记录、音量）换了存储格式，第一次刷 Rust 固件会回到默认值。
- `voice.begin` 会立刻打断正在播的那一句再擦分区；C 固件是等它播完（最多 10 秒）。
- JSON 解析比 cJSON 严格：非法 UTF-8、`"version":1.0`、非字符串的 `message` 都回 `ERROR invalid_message`（或 `invalid_json`）。Mac 端从不发这些，但 `device.echo` 查串口错字节时，收坏的那一行不再回 CRC，只回 `ERROR invalid_json`——这本身也说明串口收错了。

## 步骤

1. **先退出 Vibe Buddy App**，让出串口。
2. **接原生 USB 口**（`USB-SLAVE`，`/dev/cu.usbmodem…`），刷新固件：

   ```bash
   just flash /dev/cu.usbmodemXXXX
   ```

   它会先构建三件套，再依次写 bootloader、分区表、app。`voices` 分区与设置区不动。
3. **看开机报告。** 这一步和第 4 步都会占用串口，看完先 Ctrl-C 退出：

   ```bash
   espflash monitor -S --chip esp32s3 --port /dev/cu.usbmodemXXXX
   ```

   应当依次出现：

   ```
   TALLY LOADED 0 0S DAY 0
   DISPLAY READY BUILD v0.2.1-… 2026-…
   VOICES <原来的音色>
   AUDIO READY
   AUDIO CODEC ES8311
   VOLUME 65
   BUTTONS READY
   READY vibebuddy-fw 0.1.0
   ```

   - 第一次刷 Rust 固件时，当日番茄记录和音量回到默认值（0 次、65），这是预期的：存储格式换了。
   - 如果 panic，esp-backtrace 会先把回溯打在同一个口上，然后软复位（和 C 固件一样），所以会看到开机报告反复出现。
4. **跑冒烟检查**，按提示看屏幕、听声音：

   ```bash
   uv run --with pyserial python tools/firmware-smoke.py /dev/cu.usbmodemXXXX
   ```

5. **按键。**
   - K0 短按开始番茄钟，长按放弃。
   - K1 短按切换值班和番茄钟，长按进休闲。
   - K2 长按静音，左上角出现 MUTE。
6. **打开 App**：设备页应显示同一个固件构建号。用 Claude Code 或 Codex 跑一个任务，看任务卡、表情和播报。
7. **在 App 里换一次音色**：写完盒子会用新音色说一句。再重启盒子，音色应保持不变。
8. **只接 UART 桥过一夜。** USB 口不接主机，确认第二天早上画面还在动。这条专门验证「没有对端的输出通道不能阻塞」。

## 退回 C 固件

```bash
just flash-c /dev/cu.usbmodemXXXX
```

C 固件的 NVS 认不出 Rust 固件写的设置记录，会自己擦掉重来；语音包不受影响。

## 验收通过之后

- 删掉 `firmware/`：C 源码、`host_tests`、`tools/*-c.sh` 与 `tools/test-*.sh`、`tools/compare-display.sh`、`tools/preview-display.sh`。
- 内置 PCM 从 `firmware/main/assets` 挪到 `firmware-rs/device/assets`。
- 把 README 里 C 固件相关的段落去掉。
