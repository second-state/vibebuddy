#pragma once

#include <stdbool.h>

#include "esp_err.h"

typedef enum {
  AGENT_AUDIO_INPUT_REQUIRED,
  AGENT_AUDIO_DONE,
  AGENT_AUDIO_FAILED,
  /// 番茄钟：专注结束、休息结束各播一次。
  AGENT_AUDIO_FOCUS_DONE,
  AGENT_AUDIO_BREAK_DONE,
} agent_audio_prompt_t;

/// 音量：codec 的 0 到 100 刻度。下限不到零——能存下来的零音量就是从后门
/// 做出来的持久静音，而静音有意不持久化（见 vibebuddy_fw.c）。
#define AGENT_AUDIO_VOLUME_MIN 20u
#define AGENT_AUDIO_VOLUME_MAX 100u
#define AGENT_AUDIO_VOLUME_DEFAULT 65u

esp_err_t agent_audio_init(void);
esp_err_t agent_audio_play(agent_audio_prompt_t prompt);
/// 设置音量并存进 NVS，越界的值收进范围内；没有 codec 的板子返回
/// ESP_ERR_NOT_SUPPORTED。
esp_err_t agent_audio_set_volume(unsigned level);
unsigned agent_audio_volume(void);
const char *agent_audio_status(void);
/// 正在往 I2S 写一句：语音包写入前要等它放完。
bool agent_audio_playing(void);
