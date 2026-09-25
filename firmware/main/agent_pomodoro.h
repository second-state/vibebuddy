#pragma once

#include <stdbool.h>
#include <stdint.h>

/// Pomodoro: 25 minutes of focus, 5 minutes of break. Durations are fixed for now.
#define AGENT_POMODORO_FOCUS_MS (25u * 60u * 1000u)
#define AGENT_POMODORO_BREAK_MS (5u * 60u * 1000u)

typedef enum {
  AGENT_POMODORO_FOCUS,
  AGENT_POMODORO_BREAK,
} agent_pomodoro_phase_t;

/// Every phase starts with a key press, never automatically: after focus ends it waits
/// at "break ready", and after the break ends at "focus ready" (that is, idle).
typedef enum {
  AGENT_POMODORO_PENDING,
  AGENT_POMODORO_RUNNING,
  AGENT_POMODORO_PAUSED,
} agent_pomodoro_run_t;

/// A phase end is an edge consumed once: voice and mode switches hang off it, so the
/// same end must not fire twice.
typedef enum {
  AGENT_POMODORO_NOTHING,
  AGENT_POMODORO_FOCUS_ENDED,
  AGENT_POMODORO_BREAK_ENDED,
} agent_pomodoro_transition_t;

/// Today's record: completed focus sessions and total focus seconds. The date comes from
/// the Mac's heartbeat and a change resets it; it is restored from storage after a
/// restart. Only completed focus sessions count, abandoned ones don't.
typedef struct {
  /// Local date as YYYYMMDD; 0 means the Mac hasn't told us today's date yet.
  uint32_t day;
  unsigned completed;
  uint32_t focus_s;
} agent_pomodoro_tally_t;

typedef struct {
  agent_pomodoro_phase_t phase;
  agent_pomodoro_run_t run;
  /// Milliseconds left in the current phase; the phase's full length while waiting to start.
  uint32_t remaining_ms;
  /// Full length of the current phase, the denominator when drawing the ring.
  uint32_t total_ms;
  /// Focus sessions completed today and total focus seconds.
  unsigned completed;
  uint32_t focus_s;
} agent_pomodoro_view_t;

/// Idle: a focus session not yet started.
static inline bool agent_pomodoro_is_idle(const agent_pomodoro_view_t *view) {
  return view->phase == AGENT_POMODORO_FOCUS &&
         view->run == AGENT_POMODORO_PENDING;
}

/// This module reads no clock; every entry point takes a millisecond count from the caller, and wraparound is allowed.
void agent_pomodoro_init(void);
/// Short press: start the phase when waiting; pause when running; resume when paused.
void agent_pomodoro_toggle(uint32_t now_ms);
/// Long press: abandon the current phase and return to idle. Pressed while the break is
/// waiting to start, it skips the break. Completed counts are unaffected.
void agent_pomodoro_stop(void);
/// Advances the clock. Returns the matching transition right when a phase ends, NOTHING afterwards.
agent_pomodoro_transition_t agent_pomodoro_tick(uint32_t now_ms);
void agent_pomodoro_view(uint32_t now_ms, agent_pomodoro_view_t *view);

/// Restores today's record from storage after a restart.
void agent_pomodoro_restore_tally(const agent_pomodoro_tally_t *tally);
/// The local date from a heartbeat. If it differs from the recorded date, reset and store
/// the new date; returns true when the record changed and should be saved. 0 means
/// unknown and is ignored.
bool agent_pomodoro_set_day(uint32_t day);
void agent_pomodoro_tally(agent_pomodoro_tally_t *tally);
