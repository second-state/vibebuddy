#include "agent_pomodoro.h"

// 只依赖标准头，不碰 FreeRTOS 或 ESP-IDF：这个状态机要能在 Mac 上直接
// 编译跑测试（见 tools/test-pomodoro.sh）。

static agent_pomodoro_phase_t phase;
static agent_pomodoro_run_t run;
/// 运行中：阶段截止的毫秒计数。
static uint32_t deadline_ms;
/// 暂停中：剩余毫秒。暂停时把时间冻结成一个数，恢复时再展开成截止时刻。
static uint32_t remaining_ms;
static unsigned completed;

static uint32_t phase_length(agent_pomodoro_phase_t which) {
  return which == AGENT_POMODORO_BREAK ? AGENT_POMODORO_BREAK_MS
                                       : AGENT_POMODORO_FOCUS_MS;
}

/// 用有符号差值比较，让毫秒计数回绕时仍然正确。
static uint32_t remaining_while_running(uint32_t now_ms) {
  int32_t left = (int32_t)(deadline_ms - now_ms);
  return left > 0 ? (uint32_t)left : 0u;
}

void agent_pomodoro_init(void) {
  phase = AGENT_POMODORO_FOCUS;
  run = AGENT_POMODORO_PENDING;
  completed = 0;
}

void agent_pomodoro_toggle(uint32_t now_ms) {
  if (run == AGENT_POMODORO_PENDING) {
    run = AGENT_POMODORO_RUNNING;
    deadline_ms = now_ms + phase_length(phase);
  } else if (run == AGENT_POMODORO_PAUSED) {
    run = AGENT_POMODORO_RUNNING;
    deadline_ms = now_ms + remaining_ms;
  } else {
    run = AGENT_POMODORO_PAUSED;
    remaining_ms = remaining_while_running(now_ms);
  }
}

void agent_pomodoro_stop(void) {
  phase = AGENT_POMODORO_FOCUS;
  run = AGENT_POMODORO_PENDING;
}

agent_pomodoro_transition_t agent_pomodoro_tick(uint32_t now_ms) {
  if (run != AGENT_POMODORO_RUNNING) {
    return AGENT_POMODORO_NOTHING;
  }
  if ((int32_t)(now_ms - deadline_ms) < 0) {
    return AGENT_POMODORO_NOTHING;
  }
  // 下一阶段停在待开始，等用户按键：休息什么时候开始、下一段专注什么
  // 时候开始，都是用户的决定。
  run = AGENT_POMODORO_PENDING;
  if (phase == AGENT_POMODORO_FOCUS) {
    completed++;
    phase = AGENT_POMODORO_BREAK;
    return AGENT_POMODORO_FOCUS_ENDED;
  }
  phase = AGENT_POMODORO_FOCUS;
  return AGENT_POMODORO_BREAK_ENDED;
}

void agent_pomodoro_view(uint32_t now_ms, agent_pomodoro_view_t *view) {
  view->phase = phase;
  view->run = run;
  view->completed = completed;
  view->total_ms = phase_length(phase);
  if (run == AGENT_POMODORO_PENDING) {
    view->remaining_ms = view->total_ms;
  } else if (run == AGENT_POMODORO_PAUSED) {
    view->remaining_ms = remaining_ms;
  } else {
    view->remaining_ms = remaining_while_running(now_ms);
  }
}
