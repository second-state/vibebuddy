#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

/// 语音包：一个播报音色的五句成品打成的一包，写在设备的 `voices` 分区里。
/// 这是 Mac 与固件唯一共享的字节布局，两边各自对同一份样例做测试。
///
/// 包头 256 字节，小端序：
///   0   魔数 "VBVP"
///   4   u32 版本，现为 1
///   8   u32 载荷长度（包头之后的字节数）
///   12  u32 载荷 CRC32（zlib 同款）
///   16  char[32] 音色 id，NUL 结尾
///   48  u32[5] 各句相对包起点的偏移
///   68  u32[5] 各句长度
///   88  u32 包头前 88 字节的 CRC32
///   92  以下补零
/// 五句顺序固定：需要确认、任务完成、任务遇到问题、专注结束、休息结束。
#define AGENT_VOICE_PACK_HEADER_BYTES 256u
#define AGENT_VOICE_PACK_CLIPS 5u
#define AGENT_VOICE_PACK_ID_BYTES 32u

typedef struct {
  char voice_id[AGENT_VOICE_PACK_ID_BYTES];
  uint32_t payload_length;
  uint32_t payload_crc32;
  uint32_t clip_offset[AGENT_VOICE_PACK_CLIPS];
  uint32_t clip_length[AGENT_VOICE_PACK_CLIPS];
} agent_voice_pack_t;

/// 解析并校验包头。`capacity` 是存放区的总大小；魔数、版本、包头 CRC、
/// 任一句越界都算无效。
bool agent_voice_pack_parse(const uint8_t *header, size_t capacity,
                            agent_voice_pack_t *out);

/// zlib 同款 CRC32，可分段累计：第一段传 0，后面传上一段的结果。
uint32_t agent_voice_pack_crc32(uint32_t crc, const uint8_t *data,
                                size_t length);
