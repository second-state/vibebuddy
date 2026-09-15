# 小灯灵语音资产

PCM 文件由 macOS 的 `Tingting` 中文系统语音生成，再转换为 24 kHz、16-bit、双声道、小端序 PCM。它们只在以下状态切换时播放一次：

- `input_required.pcm`：需要你确认
- `done.pcm`：任务完成
- `failed.pcm`：任务遇到问题
- `focus_done.pcm`：钟声，然后“专注结束，休息一下”
- `break_done.pcm`：钟声，然后“休息结束”

工作中和空闲状态不播放语音，避免持续打扰用户。番茄钟的开始、暂停、继续、放弃也不出声。

前三个文件以统一增益归一化到约 90% 满量程（2026-09-14，+5.5 dB，无削波）。合成后的原始素材峰值只有 41~48%，白白浪费了 6 dB 以上的动态范围。使用统一增益而非逐个归一化，是为了保持它们之间的相对响度。要再提高响度应优先调整 `agent_audio.c` 的 codec 音量，不要把素材推到接近满量程，那会让功放削波。

番茄钟的两个文件（2026-09-15）各自归一化到 -1 dBFS 峰值：这一批 `say` 的输出峰值差了 3.5 dB，统一增益要么让一个削波、要么让另一个太轻。生成步骤：

```bash
say -v Tingting -o focus_done.aiff "专注结束，休息一下"
# 单声道转双声道会损失 3 dB，增益按实测峰值补到 -1 dBFS
ffmpeg -i focus_done.aiff -af "volume=5.4dB" -ar 24000 -ac 2 -f s16le -acodec pcm_s16le focus_voice.pcm
tools/make-pomodoro-audio.py focus focus_voice.pcm firmware/main/assets/focus_done.pcm
```

`break_done` 同理，语音是“休息结束”，增益 8.9 dB。钟声由脚本加法合成，不使用任何第三方音频素材。
