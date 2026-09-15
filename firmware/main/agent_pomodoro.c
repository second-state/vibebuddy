#include "agent_pomodoro.h"

// 只依赖标准头，不碰 FreeRTOS 或 ESP-IDF：这个状态机要能在 Mac 上直接
// 编译跑测试（见 tools/test-pomodoro.sh）。

// 实机验收用的时间压缩（见 main/CMakeLists.txt）；正式固件不定义它。
#ifndef AGENT_TIME_SCALE
#define AGENT_TIME_SCALE 1u
#endif

static agent_pomodoro_phase_t phase;
static agent_pomodoro_run_t run;
/// 运行中：阶段截止的毫秒计数。
static uint32_t deadline_ms;
/// 暂停中：剩余毫秒。暂停时把时间冻结成一个数，恢复时再展开成截止时刻。
static uint32_t remaining_ms;
static agent_pomodoro_tally_t tally;

static uint32_t phase_length(agent_pomodoro_phase_t which) {
  uint32_t full = which == AGENT_POMODORO_BREAK ? AGENT_POMODORO_BREAK_MS
                                                : AGENT_POMODORO_FOCUS_MS;
  return full / AGENT_TIME_SCALE;
}

/// 用有符号差值比较，让毫秒计数回绕时仍然正确。
static uint32_t remaining_while_running(uint32_t now_ms) {
  int32_t left = (int32_t)(deadline_ms - now_ms);
  return left > 0 ? (uint32_t)left : 0u;
}

void agent_pomodoro_init(void) {
  phase = AGENT_POMODORO_FOCUS;
  run = AGENT_POMODORO_PENDING;
  tally.day = 0;
  tally.completed = 0;
  tally.focus_s = 0;
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
    tally.completed++;
    // 记的是完整的一段专注，不是压缩后的长度：验收固件跑 25 秒也算 25 分钟。
    tally.focus_s += AGENT_POMODORO_FOCUS_MS / 1000u;
    phase = AGENT_POMODORO_BREAK;
    return AGENT_POMODORO_FOCUS_ENDED;
  }
  phase = AGENT_POMODORO_FOCUS;
  return AGENT_POMODORO_BREAK_ENDED;
}

void agent_pomodoro_view(uint32_t now_ms, agent_pomodoro_view_t *view) {
  view->phase = phase;
  view->run = run;
  view->completed = tally.completed;
  view->focus_s = tally.focus_s;
  view->total_ms = phase_length(phase);
  if (run == AGENT_POMODORO_PENDING) {
    view->remaining_ms = view->total_ms;
  } else if (run == AGENT_POMODORO_PAUSED) {
    view->remaining_ms = remaining_ms;
  } else {
    view->remaining_ms = remaining_while_running(now_ms);
  }
}

void agent_pomodoro_restore_tally(const agent_pomodoro_tally_t *restored) {
  tally = *restored;
}

bool agent_pomodoro_set_day(uint32_t day) {
  if (day == 0 || day == tally.day) {
    return false;
  }
  // 换日：清零。第一次听说日期时记录本来就是空的，清零也无妨；
  // 若重启后恢复的是昨天的记录，正好在这里归零。
  tally.day = day;
  tally.completed = 0;
  tally.focus_s = 0;
  return true;
}

void agent_pomodoro_tally(agent_pomodoro_tally_t *out) { *out = tally; }
