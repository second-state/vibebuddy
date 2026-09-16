// 语音包格式的主机测试：Mac 打出来的包，固件必须按同一套字节布局认出来。
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "agent_voice_pack.h"

static int failures;

#define CHECK(condition)                                                  \
  do {                                                                    \
    if (!(condition)) {                                                   \
      failures++;                                                         \
      fprintf(stderr, "%s:%d: 失败: %s\n", __FILE__, __LINE__, #condition); \
    }                                                                     \
  } while (0)

static void put_u32(uint8_t *at, uint32_t value) {
  at[0] = (uint8_t)value;
  at[1] = (uint8_t)(value >> 8);
  at[2] = (uint8_t)(value >> 16);
  at[3] = (uint8_t)(value >> 24);
}

/// 手工按规格摆一个包头：五段紧挨着排在包头之后。
static void build_header(uint8_t *header, const char *voice_id,
                         const uint32_t *lengths, uint32_t payload_crc) {
  memset(header, 0, AGENT_VOICE_PACK_HEADER_BYTES);
  memcpy(header, "VBVP", 4);
  put_u32(header + 4, 1);
  uint32_t total = 0;
  for (int i = 0; i < 5; i++) {
    total += lengths[i];
  }
  put_u32(header + 8, total);
  put_u32(header + 12, payload_crc);
  strncpy((char *)header + 16, voice_id, 31);
  uint32_t offset = AGENT_VOICE_PACK_HEADER_BYTES;
  for (int i = 0; i < 5; i++) {
    put_u32(header + 48 + i * 4, offset);
    put_u32(header + 68 + i * 4, lengths[i]);
    offset += lengths[i];
  }
  put_u32(header + 88, agent_voice_pack_crc32(0, header, 88));
}

static void a_well_formed_header_parses(void) {
  uint8_t header[AGENT_VOICE_PACK_HEADER_BYTES];
  const uint32_t lengths[5] = {1000, 2000, 3000, 4000, 5000};
  build_header(header, "wanwanxiaohe", lengths, 0x12345678);

  agent_voice_pack_t pack;
  CHECK(agent_voice_pack_parse(header, 2 * 1024 * 1024, &pack));
  CHECK(strcmp(pack.voice_id, "wanwanxiaohe") == 0);
  CHECK(pack.payload_length == 15000);
  CHECK(pack.payload_crc32 == 0x12345678);
  CHECK(pack.clip_offset[0] == 256);
  CHECK(pack.clip_length[0] == 1000);
  CHECK(pack.clip_offset[4] == 256 + 10000);
  CHECK(pack.clip_length[4] == 5000);
}

static void crc32_matches_zlib(void) {
  // 标准校验向量，与 Python 的 zlib.crc32 一致。
  CHECK(agent_voice_pack_crc32(0, (const uint8_t *)"123456789", 9) ==
        0xCBF43926u);
  // 分两段累计得到同一个值。
  uint32_t first = agent_voice_pack_crc32(0, (const uint8_t *)"1234", 4);
  CHECK(agent_voice_pack_crc32(first, (const uint8_t *)"56789", 5) ==
        0xCBF43926u);
}

static void a_wrong_magic_or_version_is_rejected(void) {
  uint8_t header[AGENT_VOICE_PACK_HEADER_BYTES];
  const uint32_t lengths[5] = {10, 10, 10, 10, 10};
  agent_voice_pack_t pack;

  build_header(header, "x", lengths, 0);
  header[0] = 'X';
  CHECK(!agent_voice_pack_parse(header, 4096, &pack));

  build_header(header, "x", lengths, 0);
  put_u32(header + 4, 2);
  put_u32(header + 88, agent_voice_pack_crc32(0, header, 88));
  CHECK(!agent_voice_pack_parse(header, 4096, &pack));
}

static void a_corrupted_header_is_rejected(void) {
  uint8_t header[AGENT_VOICE_PACK_HEADER_BYTES];
  const uint32_t lengths[5] = {10, 10, 10, 10, 10};
  build_header(header, "x", lengths, 0);
  header[20] ^= 0x01;  // 改动音色 id 里的一个字节，包头 CRC 不再对得上
  agent_voice_pack_t pack;
  CHECK(!agent_voice_pack_parse(header, 4096, &pack));
}

static void a_clip_outside_the_pack_is_rejected(void) {
  uint8_t header[AGENT_VOICE_PACK_HEADER_BYTES];
  const uint32_t lengths[5] = {10, 10, 10, 10, 10};
  agent_voice_pack_t pack;

  // 载荷比存放区还大。
  build_header(header, "x", lengths, 0);
  CHECK(!agent_voice_pack_parse(header, 300, &pack));

  // 某一句伸到了载荷之外。
  build_header(header, "x", lengths, 0);
  put_u32(header + 68 + 4 * 4, 11);
  put_u32(header + 88, agent_voice_pack_crc32(0, header, 88));
  CHECK(!agent_voice_pack_parse(header, 4096, &pack));

  // 某一句伸进了包头。
  build_header(header, "x", lengths, 0);
  put_u32(header + 48, 100);
  put_u32(header + 88, agent_voice_pack_crc32(0, header, 88));
  CHECK(!agent_voice_pack_parse(header, 4096, &pack));
}

int main(void) {
  a_well_formed_header_parses();
  crc32_matches_zlib();
  a_wrong_magic_or_version_is_rejected();
  a_corrupted_header_is_rejected();
  a_clip_outside_the_pack_is_rejected();
  if (failures != 0) {
    fprintf(stderr, "%d 处失败\n", failures);
    return EXIT_FAILURE;
  }
  printf("语音包格式测试通过\n");
  return EXIT_SUCCESS;
}
