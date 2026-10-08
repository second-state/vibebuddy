# 正点原子 BOX 音频器件调查

调查日期：2026-09-18。对象是用户照片中的 ATK-DNESP32S3-BOX V1.1，不是乐鑫 ESP-BOX，也不是正点原子 BOX3。

## 当前结论

**尚未取得老款 BOX 麦克风和扬声器的完整可采购料号。不能据此冻结音频 BOM 或封装。**

| 项目 | 已有证据 | 未确认内容 |
| --- | --- | --- |
| 麦克风 | 原商品资料称“数字麦克风”；照片可见独立收音器件 | 厂家、完整料号、接口、电压、灵敏度、封装与声孔方向 |
| 扬声器 | 用户照片中为板载圆形喇叭 | 厂家、完整料号、标称阻抗、额定功率、尺寸和安装方式 |
| 音频芯片 | 项目已有正点原子实机记录为 ES8311，见 `hardware.md` | 不能由编解码器型号推出麦克风或扬声器料号 |

“数字麦克风”是商品功能描述，不能据此直接认定为 INMP441、PDM 或某个 I²S 器件。面包板麦克风录音成功也不能证明其与 BOX 同料。

## 资料追踪

1. [正点原子官方旧款产品文章](https://www.cnblogs.com/zdyz/p/18715431)链接到用户同款商品与老款资料入口。对应[商品链接](https://detail.tmall.com/item.htm?id=850275706131)的资料没有给出可确认的麦克风、扬声器完整料号。
2. 老款资料入口 `http://www.openedv.com/docs/boards/esp32/ATK-DNESP32S3BVXX.html` 与论坛 `https://openedv.com/thread-350585-1-1.html` 本次无法取得资料正文；论坛浏览出现证书问题。
3. [21ic 原理图条目](https://dl.21ic.com/download/atk_dnesp32s3b_v1-756799.html)列出 `ATK_DNESP32S3B_V1.0.pdf`，679 KB，下载需要 2 积分。本次只读到文件条目，未取得 PDF。它既不是已阅读的原理图证据，也不能证明与用户 V1.1 完全一致。
4. [官方 BOX3 录音实验](https://wiki.alientek.com/docs/Boards/IoT/DNESP32S3B3/example-idf/recoding/)写明 ES8311 输出、ES7210 采集；属于不同硬件版本，不可移植为老款器件结论。
5. [官方 BOX3 音乐实验](https://wiki.alientek.com/docs/Boards/IoT/DNESP32S3B3/example-idf/music/)前言出现 NS4168，后续资源与正文又介绍 ES8311。说明文字存在不一致，不能只摘一个芯片名建立 BOM。

## 官方 GitHub 仓库核查（2026-09-18 补充）

通过 GitHub API 分页检查 `openedv` 公开仓库，并读取相关仓库的 README 与文件树：

| 入口 | 实际内容 | 对当前 BOX 的适用范围 |
| --- | --- | --- |
| [openedv 官方账号](https://github.com/openedv) | 账号介绍为广州市星翼电子，链接 alientek.com 与 wiki.alientek.com | 官方仓库总入口 |
| [ATK-DNESP32S3-Board](https://github.com/openedv/ATK-DNESP32S3-Board) | 包含 `1_docs/1_sch/ATK_DNESP32S3 V1.2.pdf`、数据手册、IDF/Arduino/MicroPython 例程；README 标明 ES8388、五个按键等资源 | 是大开发板，不是用户的 BOX V1.1，不能直接套用其音频器件 |
| [openedv-wiki-boards-dnesp32s3b3](https://github.com/openedv/openedv-wiki-boards-dnesp32s3b3) | BOX3 在线教程源文件，包含录音、播放及原理图插图；下载页仍指向 openedv.com 资料中心 | 可作新版电路参考，不能证明老款同料 |
| [openedv-wiki](https://github.com/openedv/openedv-wiki) | 当前完整文件树中 ESP32 相关产品目录为 `docs/Boards/IoT/DNESP32S3B3` | 本次未找到老款 BOX 教程目录 |
| [ATK-MD0240 屏幕仓库](https://github.com/alientek-module/LCD-module_2.4-inch_ATK-MD0240) | 独立 2.4 英寸屏幕资料，已保存规格书和原理图 | 见屏幕调查；不能用于推断音频型号 |

本次没有找到老款 BOX V1.1 的官方公开 BOM 或独立 GitHub 硬件仓库。这是当前检索结果，不是断言厂家从未公开。已找到的公开仓库说明可以直接获得部分参考资料，但尚未解决老款麦克风与扬声器的完整料号。

## 对自研 PCB 的影响

### BOX3 官方原理图核查结果

已直接读取官方仓库原理图插图，固定版本为 `c6f9797b64aef2666547206b4b274017d6d28a3d`，原文件保存在 `hardware-reference/atk-box3/`。

| 器件/电路 | 图纸证据 | 结论与限制 |
| --- | --- | --- |
| MK1 麦克风 | 采集图标注 `4015`，两端经电容接 ES7210 MIC1P/MIC1N，并有 MICBIAS12 偏置网络 | 是模拟麦克风电路；`4015` 不是包含厂家、灵敏度等信息的完整可采购料号，不能仅凭该字符串冻结封装 |
| U8 | `ES7210`；MIC3P/MIC3N 经电容及 0Ω 电阻接 CODEC_OUTP/OUTN | 同时采集麦克风和播放参考信号；官方说明用于 AEC。硬件连接本身不代表回声消除已实现 |
| U9 | `ES8311`；OUTP/OUTN 输出，麦克风输入及 ASDOUT 未接 | 此设计主要用于播放，采集由 U8 承担 |
| U10 | `NS4150B`；模拟差分输入，VoP/VoN 接 LS+/LS−，电源 VBAT | 确认功放型号；图中没有扬声器完整料号、阻抗或额定功率 |
| 扬声器 | 产品介绍写 `2520腔体喇叭` | 仅有规格类别，仍不足以作为采购料号；不能推定 4Ω、8Ω 或额定瓦数 |

来源：

- [官方采集原理图](https://github.com/openedv/openedv-wiki-boards-dnesp32s3b3/blob/c6f9797b64aef2666547206b4b274017d6d28a3d/example-idf/img/ES7210.png)
- [官方播放与功放原理图](https://github.com/openedv/openedv-wiki-boards-dnesp32s3b3/blob/c6f9797b64aef2666547206b4b274017d6d28a3d/example-idf/img/z13.png)
- [官方产品介绍](https://github.com/openedv/openedv-wiki-boards-dnesp32s3b3/blob/c6f9797b64aef2666547206b4b274017d6d28a3d/start-guide/dnesp32s3-box3-introduction.md)

### 第一版方案判断

用户在查看比较后决定第一版加入 BOX3 参考音频架构：模拟麦克风 + ES7210 采集、ES8311 播放、NS4150B 功放和外接扬声器，并保留播放参考信号采集通路。此前将单 I²S 麦克风 + MAX98357A 作为自研板基线的建议被此决定替代；面包板配置继续保留。

当前仍暂不接模型。AEC、语音打断需要后续软件、时钟同步和声学验证，不能仅凭硬件参考通路承诺效果。具体麦克风、扬声器料号和各芯片订购型号仍需落实，尚未冻结生产 BOM。

以下约束保持不变：

- 用户指定正式版采用 BOX 屏幕，但没有要求音频器件必须与 BOX 完全同料。找原装料号与选择满足需求的量产器件是两条可独立推进的路线。
- 在获得老款原理图/BOM 之前，原装麦克风与扬声器保持“待确认”，不要填写猜测的型号、阻抗或功率。
- 如果改用有明确数据手册和供应来源的音频器件，必须标为自研方案选型，并重新验证采集、播放、供电与装配；不能称为 BOX 同款。
- 保留已验证的面包板 I²S 采集和 MAX98357A 播放方案作为功能参考，见 `pcb-rev-a.md`。现有低音量试听不代替扬声器额定功率与最大音量验证。

本次仅调查资料，没有修改固件、烧录设备或生成可投产文件。
