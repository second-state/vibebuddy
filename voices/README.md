# 音色库

同一套五句提示，用五种音色各合成一遍，供日后在 macOS 端让用户挑选。每个子目录就是一整套固件资产，格式与 `firmware/main/assets/` 完全相同（24 kHz、16-bit、双声道、小端序 PCM，峰值 -1 dBFS，番茄钟两句前面拼了钟声），复制过去重新编译即可换音色。`wanwanxiaohe/` 与当前固件资产逐字节一致。

| 目录 | 音色 | 引擎 | 备注 |
|---|---|---|---|
| `wanwanxiaohe/` | 湾湾小何 `zh_female_wanwanxiaohe_moon_bigtts` | 火山引擎豆包语音 1.0（`seed-tts-1.0`） | 台湾口音，小智音箱同款，当前固件用的就是它 |
| `xiaohe2/` | 小何 2.0 `zh_female_xiaohe_uranus_bigtts` | 火山引擎豆包语音 2.0（`seed-tts-2.0`） | 普通话，同一角色的 2.0 版 |
| `hsiaoyu/` | 晓雨 `zh-TW-HsiaoYuNeural` | 微软 edge-tts | 台湾女声，不用密钥 |
| `hsiaochen/` | 晓臻 `zh-TW-HsiaoChenNeural` | 微软 edge-tts | 台湾女声，不用密钥 |
| `xiaoxiao/` | 晓晓 `zh-CN-XiaoxiaoNeural` | 微软 edge-tts | 大陆女声，不用密钥 |

五句台词固定为：需要你确认 / 任务完成 / 任务遇到问题 / 专注结束，休息一下 / 休息结束，对应 `input_required` / `done` / `failed` / `focus_done` / `break_done`。

英文版的五句是 `Need your input.` / `Task complete.` / `Task hit a problem.` / `Focus time's up. Take a break.` / `Break's over.`，由 `VOICE_LANG=en` 切换。

## 英文音色

英文音色用 ElevenLabs 合成，用的是它自带的 premade 音色（不用社区 Voice Library 里的，那些可能附带作者条款）。入库的音频必须出自付费套餐：免费套餐不含商用授权。

| 目录 | 音色 | 引擎 | 备注 |
|---|---|---|---|
| `jessica/` | Jessica `cgSgspJ2msm6clMCkdW9` | ElevenLabs（`eleven_multilingual_v2`） | 美式女声，明亮温暖 |
| `chris/` | Chris `iP95p4xoKVk53GoZ742B` | ElevenLabs（`eleven_multilingual_v2`） | 美式男声，随和自然 |

```bash
# 密钥是 ElevenLabs 控制台发的 API Key，不进仓库；先确认音色 id 在这个账号下可用
ELEVENLABS_API_KEY=... tools/elevenlabs-tts.py --list
ELEVENLABS_API_KEY=... VOICE_LANG=en OUT_DIR=voices/jessica tools/make-voices.sh
ELEVENLABS_API_KEY=... VOICE_LANG=en OUT_DIR=voices/chris ELEVENLABS_VOICE=iP95p4xoKVk53GoZ742B tools/make-voices.sh
```

目录里有了 PCM，`app/scripts/build-app.sh` 就会打出语音包，App 的音色列表随之出现这两项。换别的英文音色时，目录名要与 `app/Sources/VibeBuddyCore/VoicePack.swift` 里 `VoiceCatalogEntry.all` 的 id 一致，并在那里写上 `language: .en`。英文台词比中文长，一整套约 1.5 MB，仍在 2 MB 的 `voices` 分区以内。

火山与 ElevenLabs 的密钥都设了的时候，`make-voices.sh` 先用火山，都没设就用 edge-tts；要指定就设 `TTS_ENGINE=volc|elevenlabs|edge`。

## 重新生成

```bash
# 火山引擎音色（密钥是豆包语音控制台发的 API Key，不进仓库）
VOLC_API_KEY=... OUT_DIR=voices/wanwanxiaohe tools/make-voices.sh
VOLC_API_KEY=... OUT_DIR=voices/xiaohe2 VOLC_VOICE=zh_female_xiaohe_uranus_bigtts VOLC_RESOURCE_ID=seed-tts-2.0 tools/make-voices.sh
# edge-tts 音色
OUT_DIR=voices/hsiaochen VOICE=zh-TW-HsiaoChenNeural tools/make-voices.sh
```

## 打成语音包

写进设备 `voices` 分区的是语音包（ADR-0003），由脚本从一个音色目录打出：

```bash
tools/make_voice_pack.py voices/wanwanxiaohe wanwanxiaohe build/wanwanxiaohe.bin
```

包不入库，App 构建时现打。格式见 `firmware/main/agent_voice_pack.h`，两边各有测试：`tools/test-voice-pack.sh`（固件解析）与 `python3 tools/test_make_voice_pack.py`（打包布局）。

豆包语音 2.0 的合成结果每次略有差异，1.0 和 edge-tts 基本稳定。想试听就转成 WAV：

```bash
ffmpeg -f s16le -ar 24000 -ac 2 -i voices/hsiaochen/done.pcm done.wav
```
