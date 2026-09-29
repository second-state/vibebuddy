#pragma once

#include <stddef.h>
#include <stdint.h>

#include "agent_audio.h"
#include "esp_err.h"

/// Reading and writing voice packs: the device's `voices` partition holds the current
/// announcement voice's five finished lines. If the partition is empty or fails its
/// check, the compiled-in set (Jessica, English) is used. Changing voice writes only this
/// partition, never the firmware (ADR-0003).

/// Finds, maps and verifies the partition. Returns ESP_ERR_NOT_FOUND without one; the built-in voice still works.
esp_err_t agent_voices_init(void);

/// Current voice id; "builtin" for the built-in one.
const char *agent_voices_current_id(void);

/// Gets one line's PCM. Always succeeds: falls back to built-in when the partition has no usable pack.
void agent_voices_clip(agent_audio_prompt_t prompt, const uint8_t **data,
                       size_t *length);

/// Write session: begin erases the partition, chunk writes in order, end verifies before
/// writing the header and switching. After a failure midway or an abort the partition is
/// invalid, and announcements fall back to the built-in voice.
esp_err_t agent_voices_begin(uint32_t total_bytes);
/// `crc32` is the CRC32 of this chunk's raw bytes: one bad byte over serial is rejected on the spot, not at the end.
esp_err_t agent_voices_chunk(uint32_t seq, const char *base64,
                             size_t base64_length, uint32_t crc32);
esp_err_t agent_voices_end(void);
void agent_voices_abort(void);

/// Maximum raw bytes per chunk; the Mac splits by this so a base64 line stays under the protocol limit.
#define AGENT_VOICES_CHUNK_BYTES 672u
