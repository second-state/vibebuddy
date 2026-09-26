---
status: accepted
---

# 固件改用 Rust：esp-hal + embassy，逻辑放进可在 Mac 上测试的 firmware-core

固件原本是 ESP-IDF 上的 C（约 3900 行）。最初就打算用 Rust 写，当时没有坚持。2026-09-26 决定改写：固件改用 esp-hal 1.x 加 embassy，no_std，不带 ESP-IDF。代码分两层：

- `firmware-rs/core`：状态机、绘制、协议处理、存储格式、ES8311 与 ST7789 的命令序列，全部不碰硬件，属于 Mac 端 workspace，`cargo test` 直接跑。
- `firmware-rs/device`：只做硬件胶水，把 LCD_CAM、I2C、I2S、UART、USB Serial/JTAG、flash 接到 core 的 `Board` 接口上。它用 Xtensa 工具链单独构建。

`protocol` crate 改成 no_std 加 alloc，daemon 与固件编译同一份 `Event` 类型。

## 为什么是 esp-hal，不是 esp-idf-hal

C 固件对 ESP-IDF 的依赖很浅：

- 没用 LVGL，画面是自己画的；
- `esp_codec_dev` 只用来给 ES8311 发一串寄存器；
- NVS 只存了三个整数和一个音量；
- mbedtls 只用了 base64。

esp-idf-hal 迁移成本最低，但本质仍是 ESP-IDF，协议也还得两边各写一份。esp-hal 多出来的工作是自己写 ES8311 序列、自己写设置存储，都不大。

## 被拒绝的方案

- **保留 C，只把协议改成共享的描述文件（schema）**：解决不了「状态机与绘制只能靠 C 的主机测试凑合」的问题。
- **esp-idf-hal / esp-idf-svc**：能直接用 NVS 和 esp_codec_dev，但换来的只是一层 Rust 外壳。

## 后果

- **画面等价要靠比对来保证。** C 固件删掉之前，`tools/compare-display.sh` 渲染两份固件的同一组场景（含全部休闲剧目的每一帧）逐像素比对，改绘制代码后都要跑。首次移植时 826 帧全部一致。
- **设置存储换了格式。** 原 `nvs` 分区上改为追加式记录（`firmware-core/src/storage.rs`）。第一次刷 Rust 固件时，当日番茄记录和音量回到默认值。刷回 C 固件时，NVS 会发现页格式不认识并自行擦除。
- **分区表不变，语音包格式不变。** 换固件不丢音色。
- **构建产物的来源变了。** bootloader 用 espflash 自带的 ESP-IDF 二级 bootloader。分区表由 `tools/make-partition-table.py` 自己编，因为 espflash 解析不了自定义数据子类型，编出来的结果与 ESP-IDF 的 `gen_esp32part.py` 逐字节一致。
- **CI 换工具链。** 固件改用 esp-rs 的 Xtensa 工具链构建，不再需要 ESP-IDF 的 docker 镜像。
- **C 固件暂时保留。** `firmware/` 在实机验收完成前不删，出问题时 `just flash-c` 退回。
