#pragma once

#include <stdbool.h>
#include <stddef.h>

#include "esp_err.h"

#define AGENT_DISPLAY_MAX_TASKS 3

typedef enum {
  AGENT_DISPLAY_IDLE,
  AGENT_DISPLAY_WORKING,
  AGENT_DISPLAY_INPUT_REQUIRED,
  AGENT_DISPLAY_DONE,
  AGENT_DISPLAY_FAILED,
  AGENT_DISPLAY_OFFLINE,
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

/// 链路失联时覆盖显示：小灯灵闭眼，画面转灰。
/// 底层状态与任务卡保留，因为它们是最后已知的事实，只是不再可信。
void agent_display_set_link_lost(bool lost);
