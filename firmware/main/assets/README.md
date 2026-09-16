# 氛围小助手语音资产

PCM 文件为 24 kHz、16-bit、双声道、小端序，由 `tools/make-voices.sh` 一键生成。它们只在以下状态切换时播放一次：

- `input_required.pcm`：需要你确认
- `done.pcm`：任务完成
- `failed.pcm`：任务遇到问题
- `focus_done.pcm`：钟声，然后“专注结束，休息一下”
- `break_done.pcm`：钟声，然后“休息结束”

工作中和空闲状态不播放语音，避免持续打扰用户。番茄钟的开始、暂停、继续、放弃也不出声。

## 音色

当前音色是火山引擎豆包语音的「湾湾小何」（`zh_female_wanwanxiaohe_moon_bigtts`，1.0 模型，台湾口音，2026-09-16 更换），由 `tools/volc-tts.py` 走 V3 HTTP 接口合成。用户要的是小智音箱那种台湾女声，这个音色就是同一个。它归「语音合成 1.0」字符版，`X-Api-Resource-Id` 用 `seed-tts-1.0`；2.0 里的「小何 2.0」是普通话音色，不是替代品。

API Key 是豆包语音控制台（default 项目）自己发的那种，IAM 的 API Key 和 Access Key 都不能直接用于合成接口；密钥存在用户的凭据目录，不进仓库。之前用过 macOS 系统语音 `Tingting`（拼接式发音生硬）和微软 edge-tts 的 `zh-TW-HsiaoYuNeural`（不设 VOLC_API_KEY 时脚本仍会回落到它）。合成只在生成素材时发生，固件运行时不依赖任何在线服务。五种候选音色的完整成品都归档在仓库根目录 `voices/`，见那里的 README。

## 生成步骤

```bash
VOLC_API_KEY=... tools/make-voices.sh      # 豆包语音，默认湾湾小何
tools/make-voices.sh                       # 不给密钥则用 edge-tts HsiaoYu
```

脚本对每一句单独归一化到 -1 dBFS 峰值：不同句子的合成峰值相差可达 1.7 dB，统一增益要么让一句削波、要么让另一句太轻。单声道转双声道会损失 3 dB，脚本量的是转换之后的峰值。要再提高响度应优先调整 `agent_audio.c` 的 codec 音量，不要把素材推到满量程，那会让功放削波。

番茄钟的两个文件由 `tools/make-pomodoro-audio.py` 在语音前拼上钟声。钟声由脚本加法合成，不使用任何第三方音频素材。
