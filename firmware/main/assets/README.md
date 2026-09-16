# 氛围小助手语音资产

PCM 文件为 24 kHz、16-bit、双声道、小端序，由 `tools/make-voices.sh` 一键生成。它们只在以下状态切换时播放一次：

- `input_required.pcm`：需要你确认
- `done.pcm`：任务完成
- `failed.pcm`：任务遇到问题
- `focus_done.pcm`：钟声，然后“专注结束，休息一下”
- `break_done.pcm`：钟声，然后“休息结束”

工作中和空闲状态不播放语音，避免持续打扰用户。番茄钟的开始、暂停、继续、放弃也不出声。

## 音色

当前音色是微软神经网络语音 `zh-TW-HsiaoYuNeural`（台湾女声，2026-09-16 更换），通过 edge-tts 获取。之前用的是 macOS 系统语音 `Tingting`，拼接式发音生硬，用户要求换成类似小智音箱的台湾女声。edge-tts 走的是 Edge 浏览器“朗读”功能的接口，免费但不是正式公开的 API，只用于一次性生成这几句离线素材，固件运行时不依赖它。

若日后接火山引擎豆包的“湾湾小何”（`zh_female_wanwanxiaohe_moon_bigtts`），只需把 `make-voices.sh` 里的 `synth` 函数换成调用其 HTTP 接口，其余归一化和拼钟声的步骤不变。

## 生成步骤

```bash
tools/make-voices.sh                       # 默认 HsiaoYu
VOICE=zh-TW-HsiaoChenNeural tools/make-voices.sh   # 换音色
```

脚本对每一句单独归一化到 -1 dBFS 峰值：不同句子的合成峰值相差可达 1.7 dB，统一增益要么让一句削波、要么让另一句太轻。单声道转双声道会损失 3 dB，脚本量的是转换之后的峰值。要再提高响度应优先调整 `agent_audio.c` 的 codec 音量，不要把素材推到满量程，那会让功放削波。

番茄钟的两个文件由 `tools/make-pomodoro-audio.py` 在语音前拼上钟声。钟声由脚本加法合成，不使用任何第三方音频素材。
