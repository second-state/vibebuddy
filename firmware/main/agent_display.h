#pragma once

#include <stddef.h>

#include "esp_err.h"

#define AGENT_DISPLAY_MAX_TASKS 3

typedef enum {
  AGENT_DISPLAY_IDLE,
  AGENT_DISPLAY_WORKING,
  AGENT_DISPLAY_INPUT_REQUIRED,
  AGENT_DISPLAY_DONE,
  AGENT_DISPLAY_FAILED,
} agent_display_state_t;

typedef struct {
  const char *title;
  agent_display_state_t state;
} agent_display_task_t;

esp_err_t agent_display_init(void);
esp_err_t agent_display_show(agent_display_state_t state, const char *title);
esp_err_t agent_display_show_tasks(agent_display_state_t state,
                                   const char *title,
                                   const agent_display_task_t *tasks,
                                   size_t task_count);
void agent_display_tick(void);
