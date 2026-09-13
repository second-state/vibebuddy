# AgentBeacon Stage 0：正点原子 ESP32-S3 硬件资料核对

> 调研日期：2026-09-13  
> 证据范围：正点原子官网、官方 Wiki、官方 GitHub，以及乐鑫官方文档/仓库。第三方项目仅列在“非官方实现参考”，不参与硬件事实确认。

## 结论先行

目前**不能仅凭“正点原子 ESP32S3-BOX / ATK-MWS3S”确认底板型号，也不能据此冻结 GPIO 或 BSP**。

- 正点原子官方把“ESP32S3 开发板”和“ESP32S3 BOX”列为两个产品，但二者都采用 ATK-MWS3S；所以 ATK-MWS3S 只能识别模组，不能唯一识别底板。[正点原子官方文章](https://www.cnblogs.com/zdyz/p/18715431)
- 正点原子目前有资料完整的 **ATK-DNESP32S3B3 V1（BOX3）**。它使用 K0/K1/K2、ST7789V2、CHSC5432、ES8311、ES7210 和 NS4150B；官方简介只列一个 USB Type-C OTG 口，未列蜂鸣器或 USB-A Host。[BOX3 官方简介](https://wiki.alientek.com/docs/Boards/IoT/DNESP32S3B3/start-guide/dnesp32s3-box3-introduction/)
- 正点原子官方 `ATK-DNESP32S3-Board` 仓库对应的是 **DNESP32S3 开发板**，不是已被证明等同于 BOX 的 BSP。该板有 KEY0～KEY3、BOOT、蜂鸣器、ES8388、CH340C USB 转串口和原生 USB 从机/JTAG。[官方仓库 README](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/README.md)
- 用户描述中的“16 MB Flash、8 MB PSRAM、LCD、扬声器、麦克风、蜂鸣器、K0/K1/K2、TF、USB-C、USB-A Host、UART”与上述任一套已公开官方资料都不完全相符。尤其是“蜂鸣器 + 三按键 + USB-A Host”的组合尚无一手资料闭环。

因此 Stage 0 的硬件结论为：**型号门禁未通过**。在取得底板正反面清晰照片、PCB 丝印和接口标注之前，只能准备候选适配层，不能把任何候选板的 GPIO 当成实机事实。

## 型号歧义

### 三个不得混用的对象

| 对象 | 官方身份 | 与当前描述的关系 | 证据边界 |
|---|---|---|---|
| 正点原子 DNESP32S3 开发板 | `ATK-DNESP32S3-Board`；ATK-MWS3S，16 MB Flash、8 MB PSRAM | 有蜂鸣器、ES8388、MIC、扬声器、TF、USB 转串口和 USB 从机/JTAG；按键却是 KEY0～KEY3 + BOOT | 只能作为 DNESP32S3 开发板候选资料，不能直接套给 BOX。[官方 README](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/README.md) |
| 正点原子 BOX3 | `ATK-DNESP32S3B3 V1`；ESP32S3-R8，16 MB Flash | 有 K0/K1/K2、2.4 英寸触摸屏、MIC、扬声器、TF 和 USB-C；官方简介未列蜂鸣器、USB-A Host，也未把板卡称作 ATK-MWS3S | 只有实机确认为 `ATK-DNESP32S3B3 V1` 后，才能采用本页 BOX3 引脚。[官方简介](https://wiki.alientek.com/docs/Boards/IoT/DNESP32S3B3/start-guide/dnesp32s3-box3-introduction/) |
| 乐鑫 ESP32-S3-BOX / BOX-3 | 乐鑫自有产品线 | 名字相近，但不是正点原子板卡 | 乐鑫 BSP 只服务其自有 ESP-BOX 系列，不能作为正点原子 GPIO/BSP 依据。[乐鑫 esp-box](https://github.com/espressif/esp-box) · [乐鑫 esp-bsp 的 ESP-BOX-3](https://github.com/espressif/esp-bsp/blob/master/bsp/esp-box-3/README.md) |

正点原子官方 2025 年文章确实给老款“ESP32S3 BOX”单列了资料入口：`ATK-DNESP32S3BVXX.html`，也给 DNESP32S3 开发板单列了 `ATK-DNESP32S3.html`；这进一步证明两者不能互换。[官方文章](https://www.cnblogs.com/zdyz/p/18715431) 截至本次调研，文章内老资料入口已无法正常取得内容，因此**老款 BOX 的精确版本号、原理图与 BSP 仍未确认**。

### 当前最小可信判断

1. 若 PCB 丝印为 `ATK-DNESP32S3B3 V1`，按 BOX3 资料实现。
2. 若 PCB 丝印为 `ATK-DNESP32S3` 并与官方开发板原理图一致，按 DNESP32S3 开发板资料实现。
3. 若丝印包含 `ATK-DNESP32S3B`、`BOX` 或其他版本号，但不是 `B3 V1`，必须继续寻找该精确版本的官方资料包；不能拿 BOX3 或 DNESP32S3 的引脚表补空白。

## 官方确认：DNESP32S3 开发板候选

以下只对正点原子 `ATK-DNESP32S3-Board` 官方仓库所对应的开发板成立。引用固定在官方仓库提交 `c7434a3da5b9e6feda05added5d6a686f1c95f13`，避免后续分支变化改变证据。

### BSP、原理图与例程

- 官方仓库包含原理图、ESP-IDF/Arduino/MicroPython 例程、固件和工具。[仓库 README](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/README.md)
- 官方原理图版本为 `ATK_DNESP32S3 V1.2`。[原理图 PDF](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/1_docs/1_sch/ATK_DNESP32S3%20V1.2.pdf)
- 仓库不是乐鑫 `esp-bsp` 中的一个集中式板级包；其 ESP-IDF 例程把厂商驱动放在各例程的 `components/BSP` 下。可以抽取为 AgentBeacon 的板级适配层，但需要自行确定版本、依赖和测试边界。[I2C 扩展例程 BSP 目录](https://github.com/openedv/ATK-DNESP32S3-Board/tree/c7434a3da5b9e6feda05added5d6a686f1c95f13/2_examples/1_ESP_IDF/1_basic_routines/09_iic_exio/components/BSP)

### 按键与蜂鸣器

这块板的 KEY0～KEY3 和蜂鸣器不是直接连接 ESP32-S3 GPIO，而是连接 XL9555 I/O 扩展器；XL9555 使用 ESP32-S3 GPIO41（SDA）和 GPIO42（SCL）。[官方原理图](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/1_docs/1_sch/ATK_DNESP32S3%20V1.2.pdf)

| 功能 | XL9555 位 | 电气/软件语义 | 官方依据 |
|---|---:|---|---|
| 蜂鸣器 | P0_3 / `0x0008` | 例程写 0 开、写 1 关，即低有效 | [定义](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/2_examples/1_ESP_IDF/1_basic_routines/09_iic_exio/components/BSP/XL9555/xl9555.h) · [例程](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/2_examples/1_ESP_IDF/1_basic_routines/09_iic_exio/main/main.c) |
| KEY3 | P1_4 / `0x1000` | 低电平按下 | [定义与扫描实现](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/2_examples/1_ESP_IDF/1_basic_routines/09_iic_exio/components/BSP/XL9555/xl9555.c) |
| KEY2 | P1_5 / `0x2000` | 低电平按下 | [定义与扫描实现](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/2_examples/1_ESP_IDF/1_basic_routines/09_iic_exio/components/BSP/XL9555/xl9555.c) |
| KEY1 | P1_6 / `0x4000` | 低电平按下 | [定义与扫描实现](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/2_examples/1_ESP_IDF/1_basic_routines/09_iic_exio/components/BSP/XL9555/xl9555.c) |
| KEY0 | P1_7 / `0x8000` | 低电平按下 | [定义与扫描实现](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/2_examples/1_ESP_IDF/1_basic_routines/09_iic_exio/components/BSP/XL9555/xl9555.c) |

注意：这套官方定义没有 K0/K1/K2 命名，而是 KEY0～KEY3。若实机外壳印的是 K0/K1/K2，不能只按按键序号做映射。

### 音频

- 音频编解码器为 ES8388，板载 MIC、扬声器、耳机接口，扬声器使能通过 XL9555 的 P0_2 控制。[官方 README](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/README.md) · [XL9555 定义](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/2_examples/1_ESP_IDF/1_basic_routines/30_music/components/BSP/XL9555/xl9555.h)
- 官方音乐例程的 I2S 引脚为 MCLK=GPIO3、BCLK=GPIO46、WS/LRCK=GPIO9、ESP→codec 数据=GPIO10、codec→ESP 数据=GPIO14。[官方 I2S 头文件](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/2_examples/1_ESP_IDF/1_basic_routines/30_music/components/BSP/I2S/i2s.h)
- ES8388 通过 I2C 配置；驱动接口同时覆盖 DAC 与 ADC 路径。[官方 ES8388 头文件](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/2_examples/1_ESP_IDF/1_basic_routines/30_music/components/BSP/ES8388/es8388.h)

### LCD 与触摸

- 官方 SPI LCD 例程支持 240×240 与 320×240 两种面板配置，例程默认方向尺寸为 320×240；LCD CS=GPIO21，DC 由跳帽选择到 GPIO40，SPI MOSI/SCLK/MISO 分别为 GPIO11/GPIO12/GPIO13。[LCD 头文件](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/2_examples/1_ESP_IDF/1_basic_routines/12_spilcd/components/BSP/LCD/lcd.h) · [LCD 实现](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/2_examples/1_ESP_IDF/1_basic_routines/12_spilcd/components/BSP/LCD/lcd.c)
- 仓库包含 ST7789VW 数据手册，但通用驱动同时支持不同尺寸，因此**仅凭仓库不能确认用户 BOX 的 LCD 控制器或分辨率**。[官方资料目录](https://github.com/openedv/ATK-DNESP32S3-Board/tree/c7434a3da5b9e6feda05added5d6a686f1c95f13/1_docs/2_chip_manual)
- 官方 `touch` 例程使用 GT9xxx，并配合 RGBLCD；触摸 I2C 为 GPIO39/GPIO38，INT 为 GPIO40。它证明开发板生态中存在该触摸方案，但不能证明用户 BOX 的 2.4 英寸屏使用 GT9xxx。[GT9xxx 官方头文件](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/2_examples/1_ESP_IDF/1_basic_routines/24_touch/components/BSP/TOUCH/gt9xxx.h)

### USB、烧录与运行链路

- 原理图把 ESP32-S3 原生 USB 的 D- / D+ 分别接到 GPIO19 / GPIO20；UART0 TX/RX 分别是 GPIO43/GPIO44。[官方原理图](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/1_docs/1_sch/ATK_DNESP32S3%20V1.2.pdf)
- 该开发板提供两条烧录路径：CH340C USB 转串口，以及 ESP32-S3 原生 USB Serial/JTAG。[官方 README](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/README.md) · [官方 USB-UART 例程说明](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/2_examples/1_ESP_IDF/1_basic_routines/33_usb_uart/README.md)
- 官方 USB-UART 例程使用 TinyUSB CDC ACM，可让应用运行时通过原生 USB 提供虚拟串口。[官方实现](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/2_examples/1_ESP_IDF/1_basic_routines/33_usb_uart/main/APP/tud_usart.c)

这里必须区分三件事：CH340C 外置 UART、芯片固定功能的 USB Serial/JTAG、应用自己启动的 TinyUSB CDC。它们都可能表现为“串口”，但枚举身份、驱动、固件生命周期和故障恢复方式不同。

## 官方确认：BOX3 候选

以下仅在实机丝印确认为 `ATK-DNESP32S3B3 V1` 时成立。引用固定在官方 Wiki 源仓提交 `c6f9797b64aef2666547206b4b274017d6d28a3d`。

### 板卡组成

- 主控为 ESP32S3-R8，外接 16 MB Flash；板卡输入为 5V USB。[官方简介源文件](https://github.com/openedv/openedv-wiki-boards-dnesp32s3b3/blob/c6f9797b64aef2666547206b4b274017d6d28a3d/start-guide/dnesp32s3-box3-introduction.md)
- 屏幕为 2.4 英寸 320×240、4 线 SPI、ST7789V2；LCD CS=GPIO47、DC=GPIO48，SPI SCLK/MOSI/MISO=GPIO15/GPIO16/GPIO17。[LCD 官方说明](https://github.com/openedv/openedv-wiki-boards-dnesp32s3b3/blob/c6f9797b64aef2666547206b4b274017d6d28a3d/example-idf/lcd.md)
- 触摸控制器为 CHSC5432，I2C 7 位地址 `0x2E`。[触摸官方说明](https://github.com/openedv/openedv-wiki-boards-dnesp32s3b3/blob/c6f9797b64aef2666547206b4b274017d6d28a3d/example-idf/touch.md)
- 音频输出使用 ES8311，采集使用 ES7210，输入为模拟麦克风，功放为 NS4150B。[官方简介源文件](https://github.com/openedv/openedv-wiki-boards-dnesp32s3b3/blob/c6f9797b64aef2666547206b4b274017d6d28a3d/start-guide/dnesp32s3-box3-introduction.md)
- 音频 I2S 为 BCLK=GPIO38、WS=GPIO39、数据输出=GPIO40、数据输入=GPIO41、MCLK=GPIO21。[音乐官方说明](https://github.com/openedv/openedv-wiki-boards-dnesp32s3b3/blob/c6f9797b64aef2666547206b4b274017d6d28a3d/example-idf/music.md)

### K0/K1/K2

| 按键 | 连接 | 备注 | 官方依据 |
|---|---|---|---|
| K0 | ESP32-S3 GPIO0 | 低电平有效 | [按键例程](https://wiki.alientek.com/docs/Boards/IoT/DNESP32S3B3/example-idf/key/) |
| K1 | AW9523B P0_0 | 通过 I/O 扩展器读取 | [音乐例程源文件](https://github.com/openedv/openedv-wiki-boards-dnesp32s3b3/blob/c6f9797b64aef2666547206b4b274017d6d28a3d/example-idf/music.md) |
| K2 | AW9523B P0_1 | 通过 I/O 扩展器读取 | [音乐例程源文件](https://github.com/openedv/openedv-wiki-boards-dnesp32s3b3/blob/c6f9797b64aef2666547206b4b274017d6d28a3d/example-idf/music.md) |

BOX3 官方简介的功能列表和板载资源表均未列蜂鸣器。因此若实机确认有蜂鸣器，不能用 BOX3 文档猜测其引脚。[官方简介](https://wiki.alientek.com/docs/Boards/IoT/DNESP32S3B3/start-guide/dnesp32s3-box3-introduction/)

### USB、烧录与运行链路

- BOX3 有一个 USB Type-C OTG 接口，默认 Device 模式，用于供电、USB 通信及连接 VS Code；官方简介未列第二个 USB-A Host 口。[官方简介源文件](https://github.com/openedv/openedv-wiki-boards-dnesp32s3b3/blob/c6f9797b64aef2666547206b4b274017d6d28a3d/start-guide/dnesp32s3-box3-introduction.md)
- 官方 USB Slave 例程使用芯片原生 USB 与 TinyUSB CDC，运行时提供虚拟串口。[USB Slave 官方说明](https://github.com/openedv/openedv-wiki-boards-dnesp32s3b3/blob/c6f9797b64aef2666547206b4b274017d6d28a3d/example-idf/usb_slave.md)
- 官方烧录说明使用同一 USB 连接，并允许在开发环境中选择 UART 或 JTAG 下载方式。[烧录官方说明](https://github.com/openedv/openedv-wiki-boards-dnesp32s3b3/blob/c6f9797b64aef2666547206b4b274017d6d28a3d/set-up-development-environment/esp-flash-download.md)

## 乐鑫官方 USB 能力边界

这些是 ESP32-S3 芯片能力，不自动证明某块底板已把接口、电源和跳线按相同方式接出。

- ESP32-S3 固定功能 USB Serial/JTAG 使用 GPIO20（D+）和 GPIO19（D-），可同时提供双向串口、烧录和 JTAG；macOS 通常使用 `/dev/cu.*` 设备。[乐鑫 USB Serial/JTAG 文档](https://docs.espressif.com/projects/esp-idf/en/stable/esp32s3/api-guides/usb-serial-jtag-console.html)
- 如果应用重配 USB 引脚或禁用 USB Serial/JTAG 控制器，端口会消失；官方恢复方式包括 GPIO0 拉低后复位进入下载模式。[乐鑫 USB Serial/JTAG 文档](https://docs.espressif.com/projects/esp-idf/en/stable/esp32s3/api-guides/usb-serial-jtag-console.html)
- ESP-IDF 的 TinyUSB Device 栈可实现 CDC、HID、MIDI、MSC、Vendor 和复合设备，CDC-ACM 是应用级 USB 串口实现。[乐鑫 USB Device Stack 文档](https://docs.espressif.com/projects/esp-idf/en/stable/esp32s3/api-reference/peripherals/usb_device.html)
- 首次通过芯片原生 USB 烧录时，若自动下载未生效，官方流程是按住 BOOT、按一下 RESET，再松开 BOOT。[乐鑫串口连接文档](https://docs.espressif.com/projects/esp-idf/en/stable/esp32s3/get-started/establish-serial-connection.html)

对 AgentBeacon 而言，“一根 USB-C 同时负责供电、烧录、日志和运行期 NDJSON 串口”在芯片能力上可行，但要满足：

1. 实机 USB-C 确实连接到 GPIO19/20 的原生 USB，而不是只接 CH340C；
2. 固件选择 USB Serial/JTAG 控制台或 TinyUSB CDC 中的一条明确链路；
3. 固件不在运行中抢占/关闭该 USB 控制器；
4. macOS 上实测冷启动、烧录后重枚举、异常重启和 BOOT 恢复。

在以上实测完成前，不应把“一根线完成全部链路”写成已确认能力。

## 非官方实现参考

本节两个仓库都不是正点原子或乐鑫官方资料，**不能作为板型、GPIO、BSP、原理图或电气安全依据**。

### `second-state/echokit_box`

可借鉴：

- 用 Rust + ESP-IDF 组织设备固件，并通过 Cargo feature 选择集成式 BOX 配置；README 展示了 `espflash` 的构建/烧录流程。[项目 README](https://github.com/second-state/echokit_box/blob/4484efca885c2ffd01ffb1acdbb5817421583bd8/README.md)
- `atom_box.rs` 把屏幕、音频、按键、I2C/I/O 扩展器等集中在一个板级实现中。AgentBeacon 可以借鉴这种“业务逻辑不直接引用裸 GPIO、板型差异封装在 board profile”的结构。[板级实现](https://github.com/second-state/echokit_box/blob/4484efca885c2ffd01ffb1acdbb5817421583bd8/src/boards/atom_box.rs)
- 可参考其 framebuffer、音频任务、按钮任务的并发组织方式，但需要重新用目标板官方资料和实机测试验证时序、缓冲区与引脚。

不可作为依据：该项目自行携带/修改的 HAL 驱动和 `atom_box.rs` 中的 I2C、I2S、LCD、I/O 扩展器引脚，即使注释出现 ALIENTEK 或 ESP32S3 BOX，也只是第三方实现声明。它与上文 BOX3/DNESP32S3 的已确认引脚明显不同，恰好说明“ESP32S3 BOX”名称不足以识别硬件。

### `second-state/echokit_server`

可借鉴：设备与服务端分层、WebSocket 会话，以及 ASR→LLM→TTS 服务编排和可配置 provider 的工程组织。[项目 README](https://github.com/second-state/echokit_server/blob/d1d976596f122976095b7da4df3e946baf152b96/README.md)

不可作为依据：这是上层语音代理服务器，不提供正点原子底板的官方硬件证明。AgentBeacon v1 若采用本地守护进程 + USB NDJSON，WebSocket 与完整语音云管线也不应未经需求验证直接引入。

## 待实机/丝印确认

请一次性采集以下证据，避免继续靠商品名猜板：

1. 拆开或透过开孔拍摄主板正反面高清照片，必须能读出 PCB 型号、版本号和日期，例如是否为 `ATK-DNESP32S3B3 V1`。
2. 拍摄模组屏蔽罩丝印，确认 `ATK-MWS3S` 的具体版本；同时记录 Flash/PSRAM 的实机启动日志。
3. 拍清所有接口旁的丝印：USB-C、USB-A、UART、OTG、HOST、SLAVE、DOWNLOAD/JTAG。
4. 拍清 K0/K1/K2、蜂鸣器、MIC、扬声器、TF 及 LCD 排线/子板丝印。
5. 在 macOS 上分别插入每个 USB 口，保存 `system_profiler SPUSBDataType`、`ioreg -p IOUSB` 和 `/dev/cu.*` 变化；不要只凭接口外形判断 Host/Device。
6. 上电保存完整 ROM/bootloader 日志，记录芯片 revision、Flash、PSRAM、USB 枚举身份和固件标识。

在实机确认前保持以下项目为**未确认**：

- 精确底板型号及硬件 revision；
- LCD 控制器、分辨率、总线与背光控制；
- 触摸控制器、I2C 地址、中断/复位线；
- codec、ADC、功放、MIC 类型及所有 I2S/I2C 引脚；
- 蜂鸣器是否存在、直连还是经 I/O 扩展器、有效电平；
- K0/K1/K2 是直连 GPIO 还是扩展器位；
- USB-C 是 CH340C、USB Serial/JTAG、USB OTG/TinyUSB 中的哪条链路；
- USB-A 是否真的支持 Host、VBUS 供电能力及过流保护；
- 官方 BSP/资料包是否有对应老款 BOX 的独立版本。

## ESP-IDF 版本选择门槛

- **若确认为 BOX3**：正点原子官方环境文档明确要求其 ESP32-S3 例程使用 ESP-IDF `v5.3.x` 及以上，当前资料包示例提供 `v5.5.3` 离线安装包。[BOX3 官方安装说明](https://github.com/openedv/openedv-wiki-boards-dnesp32s3b3/blob/c6f9797b64aef2666547206b4b274017d6d28a3d/set-up-development-environment/esp-idf-install.md)
- **若确认为 DNESP32S3 开发板或老款 BOX**：已找到的 DNESP32S3 官方仓库没有声明可复现的最低/固定 ESP-IDF 版本；老款 BOX 的精确资料包又尚未取得。因此不能把 BOX3 的 `>=5.3.x` 要求外推给它们。[DNESP32S3 官方开发说明](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/1_docs/Developing_With_ESP_IDF.md)
- **AgentBeacon 的选择规则**：不要跟随 `latest` 或 `master`。先在精确板型的官方最低支持线内选择一个固定 release tag；候选版本只有在厂商原始例程与 AgentBeacon 最小探针都通过“编译、烧录、重启、USB 重枚举、LCD、触摸、按键、音频采放”后才能锁定。乐鑫也明确建议依赖 ESP-IDF 的项目先遵循该项目自己的兼容版本说明。[乐鑫 ESP-IDF 版本说明](https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-guides/tools/idf-version.html)
- **当前建议**：若实机是 BOX3，以官方资料中的 `v5.5.3` 作为第一条复现基线，再单独验证同一 `release/v5.5` 系列的最新 bugfix tag；没有完整回归前不升 ESP-IDF 6.x。若不是 BOX3，在拿到对应资料包前不冻结 IDF 版本。

## 建议下一步

1. **先过型号门禁**：以 PCB 丝印和端口照片为准，把设备归入 BOX3、DNESP32S3 开发板或“其他老款 BOX”。
2. **获取精确官方资料包**：若不是 BOX3，使用丝印型号向正点原子技术支持索要对应 revision 的原理图、例程与出厂固件，不接受相近型号替代。
3. **做只读枚举**：在 Mac Studio 上记录每个 USB 口的 VID/PID、产品名、串口节点和重枚举行为，再决定烧录/运行链路。
4. **做最小硬件探针固件**：只验证串口、三按键、蜂鸣器、LCD 色块、触摸坐标、MIC 电平、扬声器正弦波和 TF 挂载；每项先用候选 BSP，再以逻辑/示波和实机现象复核。
5. **建立板型隔离**：代码层至少区分 `dnesp32s3_devboard`、`dnesp32s3_box3` 与 `unknown_legacy_box`，型号未确认时禁止选择默认 GPIO 表。
6. **最后才冻结 AgentBeacon 接口**：硬件探针通过后，再确定一线 USB 的烧录、日志与 NDJSON 生命周期，并把经过验证的引脚表转录到 `docs/hardware.md`。
