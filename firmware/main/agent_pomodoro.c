#include "agent_pomodoro.h"

// Standard headers only, no FreeRTOS or ESP-IDF: this state machine must compile
// and run its tests directly on the Mac (see tools/test-pomodoro.sh).

// Time compression for on-device acceptance (see main/CMakeLists.txt); release firmware doesn't define it.
#ifndef AGENT_TIME_SCALE
#define AGENT_TIME_SCALE 1u
#endif

static agent_pomodoro_phase_t phase;
static agent_pomodoro_run_t run;
/// Running: the millisecond count at which the phase ends.
static uint32_t deadline_ms;
/// Paused: remaining milliseconds. Pausing freezes time into one number; resuming turns it back into a deadline.
static uint32_t remaining_ms;
static agent_pomodoro_tally_t tally;

static uint32_t phase_length(agent_pomodoro_phase_t which) {
  uint32_t full = which == AGENT_POMODORO_BREAK ? AGENT_POMODORO_BREAK_MS
                                                : AGENT_POMODORO_FOCUS_MS;
  return full / AGENT_TIME_SCALE;
}

/// Compares by signed difference so it stays correct when the millisecond count wraps.
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
  // The next phase waits to start until the user presses a key: when the break
  // starts and when the next focus starts are both the user's call.
  run = AGENT_POMODORO_PENDING;
  if (phase == AGENT_POMODORO_FOCUS) {
    tally.completed++;
    // Record a full focus session, not the compressed length: 25 seconds on acceptance firmware still counts as 25 minutes.
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
  // New day: reset. The first time we hear a date the record is empty anyway, so
  // resetting is harmless; if a restart restored yesterday's record, it resets here.
  tally.day = day;
  tally.completed = 0;
  tally.focus_s = 0;
  return true;
}

void agent_pomodoro_tally(agent_pomodoro_tally_t *out) { *out = tally; }
