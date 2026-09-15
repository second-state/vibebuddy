#pragma once

#include "agent_pomodoro.h"
#include "esp_err.h"

/// 当日番茄记录的存储：NVS 里一个命名空间，三个整数。重启不丢，换日清零。
esp_err_t agent_tally_init(void);
/// 没有记录时给出全零并返回 ESP_OK。
esp_err_t agent_tally_load(agent_pomodoro_tally_t *tally);
esp_err_t agent_tally_save(const agent_pomodoro_tally_t *tally);
