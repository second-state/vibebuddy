#pragma once

#include <stdbool.h>

#include "esp_err.h"

typedef enum {
  AGENT_AUDIO_INPUT_REQUIRED,
  AGENT_AUDIO_DONE,
  AGENT_AUDIO_FAILED,
  /// Pomodoro: played once at the end of focus and once at the end of a break.
  AGENT_AUDIO_FOCUS_DONE,
  AGENT_AUDIO_BREAK_DONE,
} agent_audio_prompt_t;

/// Volume on the codec's 0 to 100 scale. The floor is above zero: a zero volume that
/// could be saved would be a persistent mute through the back door, and mute is
/// deliberately not persisted (see vibebuddy_fw.c).
#define AGENT_AUDIO_VOLUME_MIN 20u
#define AGENT_AUDIO_VOLUME_MAX 100u
#define AGENT_AUDIO_VOLUME_DEFAULT 65u

esp_err_t agent_audio_init(void);
esp_err_t agent_audio_play(agent_audio_prompt_t prompt);
/// Sets the volume and saves it to NVS, clamping out-of-range values; boards without a codec return
/// ESP_ERR_NOT_SUPPORTED.
esp_err_t agent_audio_set_volume(unsigned level);
unsigned agent_audio_volume(void);
const char *agent_audio_status(void);
/// A line is being written to I2S: voice pack writes must wait for it to finish.
bool agent_audio_playing(void);
