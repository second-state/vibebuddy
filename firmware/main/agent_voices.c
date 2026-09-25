#include "agent_voices.h"

#include <string.h>

#include "agent_voice_pack.h"
#include "esp_log.h"
#include "esp_partition.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "mbedtls/base64.h"

extern const uint8_t
    input_required_pcm_start[] asm("_binary_input_required_pcm_start");
extern const uint8_t
    input_required_pcm_end[] asm("_binary_input_required_pcm_end");
extern const uint8_t done_pcm_start[] asm("_binary_done_pcm_start");
extern const uint8_t done_pcm_end[] asm("_binary_done_pcm_end");
extern const uint8_t failed_pcm_start[] asm("_binary_failed_pcm_start");
extern const uint8_t failed_pcm_end[] asm("_binary_failed_pcm_end");
extern const uint8_t focus_done_pcm_start[] asm("_binary_focus_done_pcm_start");
extern const uint8_t focus_done_pcm_end[] asm("_binary_focus_done_pcm_end");
extern const uint8_t break_done_pcm_start[] asm("_binary_break_done_pcm_start");
extern const uint8_t break_done_pcm_end[] asm("_binary_break_done_pcm_end");

#define FLASH_SECTOR_BYTES 4096u
/// Buffer shared by chunk decoding and read-back verification; a chunk is at most 672 bytes, verification reads 1 KB at a time.
#define CHUNK_BUFFER_BYTES 1024u
/// How long begin waits at most for the line being played to finish; the longest line is under 7 seconds.
#define AUDIO_DRAIN_MS 10000u

static const char *TAG = "agent_voices";
static const esp_partition_t *partition;
static const uint8_t *mapped;
static esp_partition_mmap_handle_t map_handle;
static agent_voice_pack_t pack;
static bool pack_loaded;
static char current_id[AGENT_VOICE_PACK_ID_BYTES] = "builtin";

/// Once a write session starts, the mapped region is no longer handed to playback. The
/// playback task marks "playing" before taking the pointer; the writer raises this flag
/// before waiting for "playing" to drop. Each side sees the other's flag, so nobody
/// feeds I2S from an address that has been unmapped.
static volatile bool pack_locked;
static bool writing;
static uint32_t expected_total;
static uint32_t received;
static uint32_t next_seq;
static uint8_t header_buffer[AGENT_VOICE_PACK_HEADER_BYTES];
static uint8_t chunk_buffer[CHUNK_BUFFER_BYTES];

static void unmap(void) {
  pack_loaded = false;
  strcpy(current_id, "builtin");
  if (mapped != NULL) {
    esp_partition_munmap(map_handle);
    mapped = NULL;
  }
}

/// Maps the partition and verifies header and payload. A failed check treats the partition as empty.
static bool map_and_validate(void) {
  unmap();
  const void *pointer;
  if (esp_partition_mmap(partition, 0, partition->size,
                         ESP_PARTITION_MMAP_DATA, &pointer,
                         &map_handle) != ESP_OK) {
    ESP_LOGW(TAG, "映射 voices 分区失败");
    return false;
  }
  mapped = pointer;
  agent_voice_pack_t parsed;
  if (!agent_voice_pack_parse(mapped, partition->size, &parsed)) {
    unmap();
    return false;
  }
  uint32_t crc = agent_voice_pack_crc32(
      0, mapped + AGENT_VOICE_PACK_HEADER_BYTES, parsed.payload_length);
  if (crc != parsed.payload_crc32) {
    ESP_LOGW(TAG, "语音包载荷 CRC 不符");
    unmap();
    return false;
  }
  pack = parsed;
  pack_loaded = true;
  strcpy(current_id, pack.voice_id);
  return true;
}

esp_err_t agent_voices_init(void) {
  partition = esp_partition_find_first(ESP_PARTITION_TYPE_DATA,
                                       ESP_PARTITION_SUBTYPE_ANY, "voices");
  if (partition == NULL) {
    return ESP_ERR_NOT_FOUND;
  }
  map_and_validate();
  return ESP_OK;
}

const char *agent_voices_current_id(void) { return current_id; }

void agent_voices_clip(agent_audio_prompt_t prompt, const uint8_t **data,
                       size_t *length) {
  __sync_synchronize();
  if (pack_loaded && !pack_locked && prompt < AGENT_VOICE_PACK_CLIPS) {
    *data = mapped + pack.clip_offset[prompt];
    *length = pack.clip_length[prompt];
    return;
  }
  const uint8_t *start = input_required_pcm_start;
  const uint8_t *end = input_required_pcm_end;
  if (prompt == AGENT_AUDIO_DONE) {
    start = done_pcm_start;
    end = done_pcm_end;
  } else if (prompt == AGENT_AUDIO_FAILED) {
    start = failed_pcm_start;
    end = failed_pcm_end;
  } else if (prompt == AGENT_AUDIO_FOCUS_DONE) {
    start = focus_done_pcm_start;
    end = focus_done_pcm_end;
  } else if (prompt == AGENT_AUDIO_BREAK_DONE) {
    start = break_done_pcm_start;
    end = break_done_pcm_end;
  }
  *data = start;
  *length = (size_t)(end - start);
}

esp_err_t agent_voices_begin(uint32_t total_bytes) {
  if (partition == NULL) {
    return ESP_ERR_NOT_FOUND;
  }
  if (total_bytes <= AGENT_VOICE_PACK_HEADER_BYTES ||
      total_bytes > partition->size) {
    return ESP_ERR_INVALID_SIZE;
  }
  // The line being played may be reading the mapped region: stop handing out new pointers first, then wait for it to finish.
  pack_locked = true;
  __sync_synchronize();
  uint32_t waited = 0;
  while (agent_audio_playing() && waited < AUDIO_DRAIN_MS) {
    vTaskDelay(pdMS_TO_TICKS(50));
    waited += 50;
  }
  unmap();
  uint32_t erase_bytes =
      (total_bytes + FLASH_SECTOR_BYTES - 1) / FLASH_SECTOR_BYTES *
      FLASH_SECTOR_BYTES;
  esp_err_t result = esp_partition_erase_range(partition, 0, erase_bytes);
  if (result != ESP_OK) {
    pack_locked = false;
    return result;
  }
  writing = true;
  expected_total = total_bytes;
  received = 0;
  next_seq = 0;
  return ESP_OK;
}

esp_err_t agent_voices_chunk(uint32_t seq, const char *base64,
                             size_t base64_length, uint32_t crc32) {
  if (!writing) {
    return ESP_ERR_INVALID_STATE;
  }
  if (seq != next_seq) {
    return ESP_ERR_INVALID_ARG;
  }
  size_t decoded = 0;
  if (mbedtls_base64_decode(chunk_buffer, sizeof(chunk_buffer), &decoded,
                            (const unsigned char *)base64,
                            base64_length) != 0 ||
      decoded == 0) {
    return ESP_ERR_INVALID_ARG;
  }
  if (received + decoded > expected_total) {
    return ESP_ERR_INVALID_SIZE;
  }
  uint32_t actual = agent_voice_pack_crc32(0, chunk_buffer, decoded);
  if (actual != crc32) {
    ESP_LOGW(TAG, "第 %lu 块 CRC 收到 %08lx，应为 %08lx", (unsigned long)seq,
             (unsigned long)actual, (unsigned long)crc32);
    return ESP_ERR_INVALID_CRC;
  }
  // The 256 header bytes stay in memory and are written to flash only after the final check passes.
  size_t consumed = 0;
  while (consumed < decoded) {
    uint32_t offset = received + (uint32_t)consumed;
    size_t remaining = decoded - consumed;
    if (offset < AGENT_VOICE_PACK_HEADER_BYTES) {
      size_t take = AGENT_VOICE_PACK_HEADER_BYTES - offset;
      if (take > remaining) {
        take = remaining;
      }
      memcpy(header_buffer + offset, chunk_buffer + consumed, take);
      consumed += take;
      continue;
    }
    esp_err_t result = esp_partition_write(partition, offset,
                                           chunk_buffer + consumed, remaining);
    if (result != ESP_OK) {
      return result;
    }
    consumed += remaining;
  }
  received += (uint32_t)decoded;
  next_seq++;
  return ESP_OK;
}

esp_err_t agent_voices_end(void) {
  if (!writing) {
    return ESP_ERR_INVALID_STATE;
  }
  writing = false;
  if (received != expected_total) {
    return ESP_ERR_INVALID_SIZE;
  }
  agent_voice_pack_t parsed;
  if (!agent_voice_pack_parse(header_buffer, partition->size, &parsed)) {
    ESP_LOGW(TAG, "语音包包头无效");
    return ESP_ERR_INVALID_RESPONSE;
  }
  if (AGENT_VOICE_PACK_HEADER_BYTES + parsed.payload_length != expected_total) {
    ESP_LOGW(TAG, "包头载荷长度 %lu 与收到的 %lu 不符",
             (unsigned long)parsed.payload_length,
             (unsigned long)(expected_total - AGENT_VOICE_PACK_HEADER_BYTES));
    return ESP_ERR_INVALID_SIZE;
  }
  // Read-back verification: what is in flash counts, not what is in memory.
  uint32_t crc = 0;
  for (uint32_t offset = 0; offset < parsed.payload_length;) {
    uint32_t block = parsed.payload_length - offset;
    if (block > CHUNK_BUFFER_BYTES) {
      block = CHUNK_BUFFER_BYTES;
    }
    esp_err_t result = esp_partition_read(
        partition, AGENT_VOICE_PACK_HEADER_BYTES + offset, chunk_buffer, block);
    if (result != ESP_OK) {
      return result;
    }
    crc = agent_voice_pack_crc32(crc, chunk_buffer, block);
    offset += block;
  }
  if (crc != parsed.payload_crc32) {
    ESP_LOGW(TAG, "载荷 CRC 回读 %08lx，包头 %08lx", (unsigned long)crc,
             (unsigned long)parsed.payload_crc32);
    return ESP_ERR_INVALID_CRC;
  }
  esp_err_t result = esp_partition_write(partition, 0, header_buffer,
                                         AGENT_VOICE_PACK_HEADER_BYTES);
  if (result != ESP_OK) {
    return result;
  }
  bool ok = map_and_validate();
  pack_locked = false;
  return ok ? ESP_OK : ESP_FAIL;
}

void agent_voices_abort(void) {
  writing = false;
  if (partition != NULL) {
    map_and_validate();
  }
  pack_locked = false;
}
