#pragma once

#include <stdbool.h>
#include <stddef.h>

#include "esp_err.h"

#define AGENT_DISPLAY_MAX_TASKS 3
#define AGENT_DISPLAY_MAX_STATS 3

typedef enum {
  AGENT_DISPLAY_IDLE,
  AGENT_DISPLAY_WORKING,
  AGENT_DISPLAY_INPUT_REQUIRED,
  AGENT_DISPLAY_DONE,
  AGENT_DISPLAY_FAILED,
  AGENT_DISPLAY_OFFLINE,
} agent_display_state_t;

/// 屏幕上同一时刻只有一个场景。小灯灵与任务卡是一个，番茄钟是另一个；
/// Agent 状态在两个场景里都继续更新，番茄钟场景只给它留一行摘要。
typedef enum {
  AGENT_SCENE_PET,
  AGENT_SCENE_POMODORO,
} agent_scene_t;

typedef struct {
  const char *title;
  agent_display_state_t state;
  /// 进入当前状态已经过去的秒数。设备收到后自行继续计时，因为可见状态
  /// 不变时 Mac 端不会再发消息，卡片上的数字却必须一直走。
  int elapsed_s;
} agent_display_task_t;

esp_err_t agent_display_init(void);
esp_err_t agent_display_show(agent_display_state_t state, const char *title);
esp_err_t agent_display_show_tasks(agent_display_state_t state,
                                   const char *title,
                                   const agent_display_task_t *tasks,
                                   size_t task_count);
void agent_display_tick(void);

void agent_display_set_scene(agent_scene_t scene);
agent_scene_t agent_display_scene(void);
/// 立即重绘。按键改变了番茄钟之后不该再等下一帧动画。
void agent_display_refresh(void);

/// 链路失联时覆盖显示：小灯灵闭眼，画面转灰。
/// 底层状态与任务卡保留，因为它们是最后已知的事实，只是不再可信。
void agent_display_set_link_lost(bool lost);

/// 记录当日战绩，空闲屏会轮播这几行。下一次绘制时生效。
void agent_display_set_stats(const char *const *lines, size_t count);

/// 设置本机固件与 Mac 端的构建标识。设备会替用户比对两者：让人去读两串
/// 哈希再自己对比并不可靠，而不一致本身正是要看见的信号。
void agent_display_set_firmware_build(const char *build);
void agent_display_set_daemon_build(const char *build);
