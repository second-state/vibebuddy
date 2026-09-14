#pragma once

#include "esp_err.h"

typedef enum {
  AGENT_DISPLAY_IDLE,
  AGENT_DISPLAY_WORKING,
  AGENT_DISPLAY_DONE,
  AGENT_DISPLAY_FAILED,
} agent_display_state_t;

esp_err_t agent_display_init(void);
esp_err_t agent_display_show(agent_display_state_t state, const char *title);
