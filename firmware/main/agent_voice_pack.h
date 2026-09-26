#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

/// Voice pack: one announcement voice's five finished lines in a single package, written
/// to the device's `voices` partition. This is the only byte layout the Mac and the
/// firmware share, and each side tests against the same sample.
///
/// 256-byte header, little endian:
///   0   magic "VBVP"
///   4   u32 version, currently 1
///   8   u32 payload length (bytes after the header)
///   12  u32 payload CRC32 (same as zlib)
///   16  char[32] voice id, NUL-terminated
///   48  u32[5] offset of each clip from the start of the pack
///   68  u32[5] length of each clip
///   88  u32 CRC32 of the header's first 88 bytes
///   92  zero padding from here on
/// The five clips are in fixed order: input required, done, failed, focus done, break done.
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

/// Parses and validates the header. `capacity` is the storage area's total size; a bad
/// magic, version or header CRC, or any clip out of bounds, makes it invalid.
bool agent_voice_pack_parse(const uint8_t *header, size_t capacity,
                            agent_voice_pack_t *out);

/// CRC32 as in zlib, accumulable in parts: pass 0 for the first part, then the previous result.
uint32_t agent_voice_pack_crc32(uint32_t crc, const uint8_t *data,
                                size_t length);
