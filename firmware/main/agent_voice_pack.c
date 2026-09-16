#include "agent_voice_pack.h"

#include <string.h>

static uint32_t read_u32(const uint8_t *at) {
  return (uint32_t)at[0] | ((uint32_t)at[1] << 8) | ((uint32_t)at[2] << 16) |
         ((uint32_t)at[3] << 24);
}

uint32_t agent_voice_pack_crc32(uint32_t crc, const uint8_t *data,
                                size_t length) {
  static uint32_t table[256];
  static bool table_ready;
  if (!table_ready) {
    for (uint32_t n = 0; n < 256; n++) {
      uint32_t c = n;
      for (int k = 0; k < 8; k++) {
        c = (c & 1) ? (0xEDB88320u ^ (c >> 1)) : (c >> 1);
      }
      table[n] = c;
    }
    table_ready = true;
  }
  uint32_t c = crc ^ 0xFFFFFFFFu;
  for (size_t i = 0; i < length; i++) {
    c = table[(c ^ data[i]) & 0xFFu] ^ (c >> 8);
  }
  return c ^ 0xFFFFFFFFu;
}

bool agent_voice_pack_parse(const uint8_t *header, size_t capacity,
                            agent_voice_pack_t *out) {
  if (memcmp(header, "VBVP", 4) != 0 || read_u32(header + 4) != 1) {
    return false;
  }
  if (read_u32(header + 88) != agent_voice_pack_crc32(0, header, 88)) {
    return false;
  }
  agent_voice_pack_t pack;
  memset(&pack, 0, sizeof(pack));
  pack.payload_length = read_u32(header + 8);
  pack.payload_crc32 = read_u32(header + 12);
  memcpy(pack.voice_id, header + 16, AGENT_VOICE_PACK_ID_BYTES - 1);
  if (pack.voice_id[0] == '\0' ||
      (uint64_t)AGENT_VOICE_PACK_HEADER_BYTES + pack.payload_length > capacity) {
    return false;
  }
  for (size_t i = 0; i < AGENT_VOICE_PACK_CLIPS; i++) {
    pack.clip_offset[i] = read_u32(header + 48 + i * 4);
    pack.clip_length[i] = read_u32(header + 68 + i * 4);
    uint64_t end = (uint64_t)pack.clip_offset[i] + pack.clip_length[i];
    if (pack.clip_length[i] == 0 ||
        pack.clip_offset[i] < AGENT_VOICE_PACK_HEADER_BYTES ||
        end > (uint64_t)AGENT_VOICE_PACK_HEADER_BYTES + pack.payload_length) {
      return false;
    }
  }
  *out = pack;
  return true;
}
