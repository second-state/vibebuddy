# 氛围小助手语音资产

PCM 文件为 24 kHz、16-bit、双声道、小端序，由 `tools/make-voices.sh` 一键生成。它们只在以下状态切换时播放一次：

- `input_required.pcm`：Hey, I need you for a sec.
- `done.pcm`：All done!
- `failed.pcm`：Uh-oh, something went wrong.
- `focus_done.pcm`：钟声，然后 “Nice work. Time for a break!”
- `break_done.pcm`：钟声，然后 “Break's over. Back to it!”

工作中和空闲状态不播放语音，避免持续打扰用户。番茄钟的开始、暂停、继续、放弃也不出声。

## 音色

当前音色是 ElevenLabs 自带的 premade 音色 Jessica（`cgSgspJ2msm6clMCkdW9`，`eleven_multilingual_v2` 模型，美式女声，2026-09-29 更换），由 `tools/elevenlabs-tts.py` 合成。项目转向英文开源，出厂听到的应当是英文；中文用户在 App 里挑一个中文音色写进 `voices` 分区即可。入库音频必须出自 ElevenLabs 付费套餐，免费套餐不含商用授权。

之前的内置音色是火山引擎豆包语音的「湾湾小何」（2026-09-16 至 2026-09-29），再之前用过 macOS 系统语音 `Tingting` 和微软 edge-tts 的 `zh-TW-HsiaoYuNeural`。湾湾小何仍作为可选音色归档在 `voices/wanwanxiaohe/`。API Key 都存在用户自己的环境里，不进仓库。合成只在生成素材时发生，固件运行时不依赖任何在线服务。全部候选音色的成品归档在仓库根目录 `voices/`，见那里的 README。

## 生成步骤

```bash
# 先生成到音色库，再复制过来：每次合成都略有差异，这样两处逐字节一致
ELEVENLABS_API_KEY=... OUT_DIR=voices/jessica tools/make-voices.sh
cp voices/jessica/*.pcm firmware/main/assets/
```

脚本对每一句单独归一化到 -1 dBFS 峰值：不同句子的合成峰值相差可达 1.7 dB，统一增益要么让一句削波、要么让另一句太轻。单声道转双声道会损失 3 dB，脚本量的是转换之后的峰值。要再提高响度应优先调整 `agent_audio.c` 的 codec 音量，不要把素材推到满量程，那会让功放削波。

番茄钟的两个文件由 `tools/make-pomodoro-audio.py` 在语音前拼上钟声。钟声由脚本加法合成，不使用任何第三方音频素材。
