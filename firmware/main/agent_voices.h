#pragma once

#include <stddef.h>
#include <stdint.h>

#include "agent_audio.h"
#include "esp_err.h"

/// 语音包的读与写：设备 `voices` 分区里放着当前播报音色的五句成品。
/// 分区为空或校验不过就用编译内置的那套（湾湾小何）。换音色只写这个
/// 分区，不换固件（ADR-0003）。

/// 找到分区并映射、校验。没有分区时返回 ESP_ERR_NOT_FOUND，内置音色照用。
esp_err_t agent_voices_init(void);

/// 当前音色 id；内置为 "builtin"。
const char *agent_voices_current_id(void);

/// 取一句的 PCM。永远成功：分区没有可用的包就回内置。
void agent_voices_clip(agent_audio_prompt_t prompt, const uint8_t **data,
                       size_t *length);

/// 写入会话：begin 擦分区，chunk 按序写入，end 校验后才写包头并切换。
/// 中途失败或 abort 之后分区无效，播报自动回落内置音色。
esp_err_t agent_voices_begin(uint32_t total_bytes);
/// `crc32` 是这一块原始字节的 CRC32：串口收错一个字节就当场拒绝，不等到最后。
esp_err_t agent_voices_chunk(uint32_t seq, const char *base64,
                             size_t base64_length, uint32_t crc32);
esp_err_t agent_voices_end(void);
void agent_voices_abort(void);

/// 每块最多这么多原始字节；Mac 端按它切块，base64 后一行不超过协议上限。
#define AGENT_VOICES_CHUNK_BYTES 672u
