#pragma once

#include "esp_err.h"

typedef enum {
  AGENT_AUDIO_INPUT_REQUIRED,
  AGENT_AUDIO_DONE,
  AGENT_AUDIO_FAILED,
  /// 番茄钟：专注结束、休息结束各播一次。
  AGENT_AUDIO_FOCUS_DONE,
  AGENT_AUDIO_BREAK_DONE,
} agent_audio_prompt_t;

esp_err_t agent_audio_init(void);
esp_err_t agent_audio_play(agent_audio_prompt_t prompt);
const char *agent_audio_status(void);
