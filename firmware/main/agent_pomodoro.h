#pragma once

#include <stdbool.h>
#include <stdint.h>

/// 番茄钟：专注 25 分钟，休息 5 分钟。时长暂时固定。
#define AGENT_POMODORO_FOCUS_MS (25u * 60u * 1000u)
#define AGENT_POMODORO_BREAK_MS (5u * 60u * 1000u)

typedef enum {
  AGENT_POMODORO_FOCUS,
  AGENT_POMODORO_BREAK,
} agent_pomodoro_phase_t;

/// 每个阶段都由用户按键开始，不自动衔接：专注结束后停在“休息待开始”，
/// 休息结束后停在“专注待开始”（也就是空闲）。
typedef enum {
  AGENT_POMODORO_PENDING,
  AGENT_POMODORO_RUNNING,
  AGENT_POMODORO_PAUSED,
} agent_pomodoro_run_t;

/// 阶段结束是只消费一次的边沿：语音与场景切换都挂在它上面，
/// 同一次结束不得触发第二遍。
typedef enum {
  AGENT_POMODORO_NOTHING,
  AGENT_POMODORO_FOCUS_ENDED,
  AGENT_POMODORO_BREAK_ENDED,
} agent_pomodoro_transition_t;

typedef struct {
  agent_pomodoro_phase_t phase;
  agent_pomodoro_run_t run;
  /// 当前阶段还剩多少毫秒；待开始时给出这一阶段的全长。
  uint32_t remaining_ms;
  /// 当前阶段的全长，画圆环时作分母。
  uint32_t total_ms;
  /// 开机以来完成的专注次数。
  unsigned completed;
} agent_pomodoro_view_t;

/// 空闲：还没开始的专注。
static inline bool agent_pomodoro_is_idle(const agent_pomodoro_view_t *view) {
  return view->phase == AGENT_POMODORO_FOCUS &&
         view->run == AGENT_POMODORO_PENDING;
}

/// 本模块不读时钟，所有入口都由调用方传入毫秒计数，允许回绕。
void agent_pomodoro_init(void);
/// 短按：待开始时开始这一阶段；运行中暂停；暂停中继续。
void agent_pomodoro_toggle(uint32_t now_ms);
/// 长按：放弃当前阶段，回到空闲。休息待开始时按它就是跳过休息。
/// 已完成的次数不受影响。
void agent_pomodoro_stop(void);
/// 推进时钟。阶段刚结束时返回对应的转换，之后返回 NOTHING。
agent_pomodoro_transition_t agent_pomodoro_tick(uint32_t now_ms);
void agent_pomodoro_view(uint32_t now_ms, agent_pomodoro_view_t *view);
