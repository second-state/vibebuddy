#pragma once

#include "agent_pomodoro.h"
#include "esp_err.h"

/// Storage for today's pomodoro record: one NVS namespace, three integers. Survives restarts, resets on a new day.
esp_err_t agent_tally_init(void);
/// With no record, yields all zeros and returns ESP_OK.
esp_err_t agent_tally_load(agent_pomodoro_tally_t *tally);
esp_err_t agent_tally_save(const agent_pomodoro_tally_t *tally);
