# 小灯灵语音资产

三个 PCM 文件由 macOS 的 `Tingting` 中文系统语音生成，再转换为 24 kHz、16-bit、双声道、小端序 PCM。它们只在以下状态切换时播放一次：

- `input_required.pcm`：需要你确认
- `done.pcm`：任务完成
- `failed.pcm`：任务遇到问题

工作中和空闲状态不播放语音，避免持续打扰用户。
