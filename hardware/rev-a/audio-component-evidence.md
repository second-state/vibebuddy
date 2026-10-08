# Rev A 音频器件与逐脚证据

日期：2026-09-18。范围：ES7210 + ES8311 + NS4150B；不是老款 BOX 原装料号认定。

## 原始来源与资料优先级

- 正点原子官方 BOX3 [ES7210 原理图](https://github.com/openedv/openedv-wiki-boards-dnesp32s3b3/blob/c6f9797b64aef2666547206b4b274017d6d28a3d/example-idf/img/ES7210.png)、[播放原理图](https://github.com/openedv/openedv-wiki-boards-dnesp32s3b3/blob/c6f9797b64aef2666547206b4b274017d6d28a3d/example-idf/img/z13.png)。下列 BOX 元件值直接读图。
- [Everest ES7210 Rev22.0 May2023 原厂手册，由 Waveshare 托管](https://files.waveshare.com/wiki/common/ES7210-datasheet.pdf)，本地 `docs/hardware-reference/audio/ES7210.pdf`。
- Everest ES8311 Rev5.0 March2019、ES7210 User Guide 2018-06-07、纳芯威 NS4150B March2021 V1.1 原厂 PDF 从[固定版本镜像](https://github.com/numuly/box3-dev/tree/3f557c7cb8c0909c22e613d2f25e7e2311e55162/raw/sources)下载到 `docs/hardware-reference/audio/`。它是第三方托管的原厂文件，不是第三方文字总结。ES7210 老 User Guide 的应用图有引脚编号偏移，**引脚以 Rev22 数据表与官方 BOX3 图交叉一致值为准**。
- [纳芯威原厂 NS4150B 产品页](https://en.nsiway.com.cn/list_38/82.html)确认 MSOP8 产品。
- [Same Sky CMC-4015-40P 原厂规格书](https://www.sameskydevices.com/product/resource/cmc-4015-40p.pdf)、[CES-28266-24-L200 原厂规格书](https://www.sameskydevices.com/product/resource/ces-28266-24-l200.pdf)，均已本地归档。

## 器件选择与封装

|器件完整名称|封装或尺寸|依据与状态|
|---|---|---|
|Everest ES7210|QFN32，4×4 mm，间距0.4 mm，裸露焊盘2.8×2.8 mm|Rev22 p1订购名、p10图；EP记33，接地|
|Everest ES8311|QFN20，3×3 mm，间距0.4 mm，裸露焊盘1.7×1.7 mm|Rev5 p1、p11；EP记21，接地|
|Nsiway NS4150B|MSOP8，主体3×3 mm，间距0.65 mm，含脚总宽典型4.9 mm|p9；不要用SOP8封装替代|
|Same Sky CMC-4015-40P|驻极体模拟麦克风，直径4 mm、高1.5 mm，插针间距1.4±0.3 mm，脚直径0.4±0.1 mm|选作明确料号样机候选；不是BOX原装认定。端1正，端2负且与金属壳相连。按手册手焊，不假定可回流|
|Same Sky CES-28266-24-L200|带腔体4Ω、额定2W、引线200mm；28×26×约6mm|选作明确料号样机候选，机械图含胶与局部突出需另核；应固定到外壳，不把扬声器本体当PCB贴片件|

麦克风原厂规格：2V标准供电、最高10V、2.2kΩ负载、最大0.5mA、灵敏度-40±3dB、58dBA SNR。原厂测量电路为电源经2.2kΩ到端1、端2接地、端1经1µF输出。麦克风应按这个选定料号的电路设计，不能因为都叫4015就照搬未知麦克风偏置网络。Mouser页面存在该料号库存，但本轮未锁定采购；不假定嘉立创标准库有货。

扬声器原厂目录有 Stock Check/Buy 路径，但本轮未锁定现货与报价。候选是为原型提供完整规格，未证明符合最终低成本目标。

## ES7210 全引脚连接表

|脚|名称|Rev A 连接原则|
|---|---|---|
|1|AD0|地，地址0x40|
|2|AD1|地，地址0x40|
|3|CDATA|公共I2C SDA，上拉3.3V|
|4|CCLK|公共I2C SCL，上拉3.3V|
|5|MCLK|ESP32主时钟|
|6|VDDP|3.3V音频数字电源|
|7|VDDD|同上|
|8|GNDD|地|
|9|SCLK|公共音频BCLK|
|10|LRCK|公共音频WS|
|11|SDOUT1/TDMOUT|ESP32录音DIN|
|12|SDOUT2/TDMIN|不用则显式NC|
|13|INT|不用则显式NC|
|14|DMIC_CLK|模拟麦克风模式不用，NC|
|15|MIC1N|经1µF接麦克风地参考|
|16|MIC1P|经1µF接麦克风正端信号|
|17|REFP12|1µF到地，紧邻芯片|
|18|REFQ12|1µF到地，紧邻芯片|
|19|MIC2P|不用，经1µF接地，软件关闭该通道|
|20|MIC2N|不用，经1µF接地|
|21|GNDA|地|
|22|VDDA|3.3V音频模拟电源|
|23|VDDM|同上|
|24|MICBIAS12|麦克风偏置，2.2µF到地|
|25|REFQM|1µF到地|
|26|MICBIAS34|1µF到地；不用于喇叭参考供电|
|27|MIC4N|不用，经1µF接地|
|28|MIC4P|不用，经1µF接地，软件关闭该通道|
|29|REFP34|1µF到地|
|30|REFQ34|1µF到地|
|31|MIC3P|ES8311 OUTP经0Ω和1µF交流耦合|
|32|MIC3N|ES8311 OUTN经0Ω和1µF交流耦合|
|33|EP|地铜与散热过孔，不能悬空|

BOX3：6/7电源经120Ω@参考频率的磁珠共用10µF+100nF，22/23另磁珠共用10µF+100nF。Rev A 应为每个电源脚增加靠近引脚的100nF，保留各域储能电容。磁珠的“120R/1A”不是120Ω串联电阻，应落实具体料号/阻抗频率/直流电阻。所有供电使用3.3V；Rev22 p7要求VDDP/VDDD上下电相差建议不超过10ms，VDDA开时VDDD必须开。

MICBIAS：User Guide p23给出VDDM=3.3V时0x41[6:4]可选2.18～2.87V，默认110为2.78V；0x4B bit6关闭MICBIAS12。选定驻极体建议采用原厂2.2kΩ负载与1µF耦合、MICBIAS初始2.78V；样机实测静态电压和录音噪声后再定增益。BOX3原图另有2kΩ/1kΩ/2kΩ与10µF、33pF的未知4015网络，不认定对本候选最优。

播放参考取 **ES8311差分模拟输出、在功放前**，不是NS4150B的D类扬声器输出。保持ADC参考通道低增益（初始0dB），核算满量程，避免麦克风高增益也套给参考通道。该线路不包含功放失真，硬件连接本身不等于AEC已验证。

## ES8311 全引脚连接表

|脚|名称|Rev A 连接原则|
|---|---|---|
|1|CCLK|I2C SCL|
|2|MCLK|共享音频MCLK|
|3|PVDD|3.3V数字音频电源|
|4|DVDD|同上|
|5|DGND|地|
|6|SCLK/DMIC_SCL|共享BCLK|
|7|ASDOUT|不使用本芯片ADC，NC|
|8|LRCK|共享WS|
|9|DSDIN|ESP32播放DOUT|
|10|AGND|地|
|11|AVDD|3.3V模拟音频电源|
|12|OUTP|分支到ES7210参考及功放正输入|
|13|OUTN|分支到ES7210参考及功放负输入|
|14|DACVREF|2.2µF到地（BOX3值；原厂典型1µF）|
|15|ADCVREF|2.2µF到地|
|16|VMID|2.2µF到地|
|17|MIC1N|播放专用，按BOX3不用NC|
|18|MIC1P/DMIC_SDA|播放专用，按BOX3不用NC|
|19|CDATA|I2C SDA|
|20|CE|地（7位I2C0x18）|
|21|EP|地|

3/4共用磁珠后10µF+100nF；11另磁珠后10µF+100nF；Rev A每电源脚附近补100nF。12/13各经2.2µF再30kΩ到NS4150B 3/4，BOX3此值对应功放增益240k/30k=8倍。原厂ES8311额定差分输出和输入不能直接由供电电压代替，PCB定稿需按播放最大设置核算幅度与限幅。

## NS4150B 全引脚连接表

|脚|名称|连接|
|---|---|---|
|1|CTRL|MCU PA_EN；建议100kΩ下拉默认关，初始化后拉高（相对于BOX上拉的有意改动）|
|2|Bypass|1µF到地（数据表p2；BOX3用2.2µF）|
|3|INP|上文OUTP耦合+30kΩ|
|4|INN|上文OUTN耦合+30kΩ|
|5|VoN|扬声器负；不能接GND|
|6|VCC|稳压音频功率电源；近端1µF+10µF或更大，另100nF可保留|
|7|GND|功率回流，不经过麦克风回流|
|8|VoP|扬声器正|

**重要电源边界**：手册正常3.0～5.0V，绝对最大5.25V；USB-C VBUS不能无条件直接作功放电源。可采用足够电流的稳压3.3V功率轨（输出功率降低，重新核算散热/瞬态）或合规独立稳压轨。不要将BOX3的VBAT标签直接换成USB5V。功放3.3V供电时3.3V MCU控制兼容。输出为桥接D类，任何一端都不是地；测试应差分，不把示波器地夹夹到VoN。

## 绘图和发布前边界

- 从Rev22和BOX图确认的引脚可以用于KiCad建符号；老User Guide应用图不可自动抽取成引脚。
- QFN32、QFN20间距都是0.4mm，不能按常见0.5mm套用。
- ES7210与ES8311共享MCLK/BCLK/WS，但需配置相容I2S/TDM时序，确定麦克风+参考两通道输出方式；这是后续固件必须验证项。
- 地布局保持连续回流并让扬声器大电流避开麦克风/参考；不要因原理图AGND标签而随意切断USB或I2S下面的地平面。
- 麦克风/扬声器为完整型号候选，库存、成本、装配（手焊）、声孔与腔体位置需在生产BOM冻结前确定。不能因此声称全套音频设计已经实机验证。
