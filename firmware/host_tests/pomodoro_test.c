// Host tests for the pomodoro state machine. Built and run by tools/test-pomodoro.sh.
#include <stdio.h>
#include <stdlib.h>

#include "agent_pomodoro.h"

static int failures;

#define CHECK(condition)                                                    \
  do {                                                                      \
    if (!(condition)) {                                                     \
      failures++;                                                           \
      fprintf(stderr, "%s:%d: 断言失败: %s\n", __FILE__, __LINE__,          \
              #condition);                                                  \
    }                                                                       \
  } while (0)

static agent_pomodoro_view_t view_at(uint32_t now_ms) {
  agent_pomodoro_view_t view;
  agent_pomodoro_view(now_ms, &view);
  return view;
}

static void idle_shows_a_full_focus(void) {
  agent_pomodoro_init();
  agent_pomodoro_view_t view = view_at(0);
  CHECK(agent_pomodoro_is_idle(&view));
  CHECK(view.remaining_ms == AGENT_POMODORO_FOCUS_MS);
  CHECK(view.total_ms == AGENT_POMODORO_FOCUS_MS);
  CHECK(view.completed == 0);
  CHECK(agent_pomodoro_tick(1000) == AGENT_POMODORO_NOTHING);
}

static void toggle_starts_pauses_and_resumes(void) {
  agent_pomodoro_init();
  agent_pomodoro_toggle(1000);
  agent_pomodoro_view_t view = view_at(2000);
  CHECK(view.phase == AGENT_POMODORO_FOCUS);
  CHECK(view.run == AGENT_POMODORO_RUNNING);
  CHECK(view.remaining_ms == AGENT_POMODORO_FOCUS_MS - 1000);

  agent_pomodoro_toggle(6000);
  view = view_at(60000);
  CHECK(view.run == AGENT_POMODORO_PAUSED);
  CHECK(view.remaining_ms == AGENT_POMODORO_FOCUS_MS - 5000);
  CHECK(agent_pomodoro_tick(60000) == AGENT_POMODORO_NOTHING);

  agent_pomodoro_toggle(60000);
  view = view_at(61000);
  CHECK(view.run == AGENT_POMODORO_RUNNING);
  CHECK(view.remaining_ms == AGENT_POMODORO_FOCUS_MS - 6000);
}

static void focus_end_waits_for_the_user_to_start_the_break(void) {
  agent_pomodoro_init();
  agent_pomodoro_toggle(0);
  CHECK(agent_pomodoro_tick(AGENT_POMODORO_FOCUS_MS - 1) ==
        AGENT_POMODORO_NOTHING);
  // A tick arriving 40 ms late still reports the end only once.
  CHECK(agent_pomodoro_tick(AGENT_POMODORO_FOCUS_MS + 40) ==
        AGENT_POMODORO_FOCUS_ENDED);
  CHECK(agent_pomodoro_tick(AGENT_POMODORO_FOCUS_MS + 80) ==
        AGENT_POMODORO_NOTHING);

  // The break doesn't start on its own: it waits to start and the clock doesn't run.
  agent_pomodoro_view_t view = view_at(AGENT_POMODORO_FOCUS_MS + 60000);
  CHECK(view.phase == AGENT_POMODORO_BREAK);
  CHECK(view.run == AGENT_POMODORO_PENDING);
  CHECK(!agent_pomodoro_is_idle(&view));
  CHECK(view.completed == 1);
  CHECK(view.total_ms == AGENT_POMODORO_BREAK_MS);
  CHECK(view.remaining_ms == AGENT_POMODORO_BREAK_MS);
  CHECK(agent_pomodoro_tick(AGENT_POMODORO_FOCUS_MS + 3600000) ==
        AGENT_POMODORO_NOTHING);

  // The break starts when the user presses the key, timed from the press.
  uint32_t break_start = AGENT_POMODORO_FOCUS_MS + 90000;
  agent_pomodoro_toggle(break_start);
  view = view_at(break_start + 1000);
  CHECK(view.run == AGENT_POMODORO_RUNNING);
  CHECK(view.remaining_ms == AGENT_POMODORO_BREAK_MS - 1000);

  uint32_t break_end = break_start + AGENT_POMODORO_BREAK_MS;
  CHECK(agent_pomodoro_tick(break_end - 1) == AGENT_POMODORO_NOTHING);
  CHECK(agent_pomodoro_tick(break_end) == AGENT_POMODORO_BREAK_ENDED);
  CHECK(agent_pomodoro_tick(break_end + 1) == AGENT_POMODORO_NOTHING);
  view = view_at(break_end + 1);
  CHECK(agent_pomodoro_is_idle(&view));
  CHECK(view.remaining_ms == AGENT_POMODORO_FOCUS_MS);
  CHECK(view.completed == 1);
}

static void stop_abandons_without_counting(void) {
  agent_pomodoro_init();
  agent_pomodoro_toggle(0);
  agent_pomodoro_stop();
  agent_pomodoro_view_t view = view_at(AGENT_POMODORO_FOCUS_MS + 1);
  CHECK(agent_pomodoro_is_idle(&view));
  CHECK(view.completed == 0);
  CHECK(agent_pomodoro_tick(AGENT_POMODORO_FOCUS_MS + 1) ==
        AGENT_POMODORO_NOTHING);

  // Abandoning after a pause must not leak the old remaining time into the next focus.
  agent_pomodoro_toggle(0);
  agent_pomodoro_toggle(1000);
  agent_pomodoro_stop();
  agent_pomodoro_toggle(5000);
  view = view_at(5000);
  CHECK(view.phase == AGENT_POMODORO_FOCUS);
  CHECK(view.run == AGENT_POMODORO_RUNNING);
  CHECK(view.remaining_ms == AGENT_POMODORO_FOCUS_MS);
}

static void stop_skips_a_pending_break(void) {
  agent_pomodoro_init();
  agent_pomodoro_toggle(0);
  CHECK(agent_pomodoro_tick(AGENT_POMODORO_FOCUS_MS) ==
        AGENT_POMODORO_FOCUS_ENDED);
  agent_pomodoro_stop();
  agent_pomodoro_view_t view = view_at(AGENT_POMODORO_FOCUS_MS + 1);
  CHECK(agent_pomodoro_is_idle(&view));
  // Skipping the break doesn't erase the focus session already completed.
  CHECK(view.completed == 1);
}

static void tally_counts_completed_focus_and_resets_by_day(void) {
  agent_pomodoro_init();
  CHECK(agent_pomodoro_set_day(20260915));
  CHECK(!agent_pomodoro_set_day(20260915));
  CHECK(!agent_pomodoro_set_day(0));

  agent_pomodoro_toggle(0);
  (void)agent_pomodoro_tick(AGENT_POMODORO_FOCUS_MS);
  agent_pomodoro_toggle(AGENT_POMODORO_FOCUS_MS);  // start the break
  (void)agent_pomodoro_tick(AGENT_POMODORO_FOCUS_MS + AGENT_POMODORO_BREAK_MS);
  agent_pomodoro_toggle(AGENT_POMODORO_FOCUS_MS + AGENT_POMODORO_BREAK_MS);
  (void)agent_pomodoro_tick(2 * AGENT_POMODORO_FOCUS_MS +
                            AGENT_POMODORO_BREAK_MS);
  agent_pomodoro_view_t view = view_at(2 * AGENT_POMODORO_FOCUS_MS +
                                       AGENT_POMODORO_BREAK_MS);
  CHECK(view.completed == 2);
  CHECK(view.focus_s == 2 * AGENT_POMODORO_FOCUS_MS / 1000u);

  // Abandoned sessions don't count.
  agent_pomodoro_stop();
  agent_pomodoro_toggle(0);
  agent_pomodoro_stop();
  agent_pomodoro_tally_t tally;
  agent_pomodoro_tally(&tally);
  CHECK(tally.completed == 2);
  CHECK(tally.day == 20260915);

  // Reset on a new day; not when the date is the same.
  CHECK(agent_pomodoro_set_day(20260916));
  agent_pomodoro_tally(&tally);
  CHECK(tally.completed == 0);
  CHECK(tally.focus_s == 0);
  CHECK(tally.day == 20260916);

  // After a restart yesterday's record is restored; it resets only when a heartbeat brings today's date.
  agent_pomodoro_init();
  agent_pomodoro_tally_t restored = {20260916, 3, 4500};
  agent_pomodoro_restore_tally(&restored);
  view = view_at(0);
  CHECK(view.completed == 3);
  CHECK(view.focus_s == 4500);
  CHECK(!agent_pomodoro_set_day(20260916));
  CHECK(view_at(0).completed == 3);
  CHECK(agent_pomodoro_set_day(20260917));
  CHECK(view_at(0).completed == 0);
}

static void millisecond_counter_may_wrap(void) {
  agent_pomodoro_init();
  uint32_t start = UINT32_MAX - 1000;
  agent_pomodoro_toggle(start);
  uint32_t wrapped = start + 5000;  // wraps around to 3999
  CHECK(wrapped == 3999);
  agent_pomodoro_view_t view = view_at(wrapped);
  CHECK(view.remaining_ms == AGENT_POMODORO_FOCUS_MS - 5000);
  CHECK(agent_pomodoro_tick(wrapped) == AGENT_POMODORO_NOTHING);
  CHECK(agent_pomodoro_tick(start + AGENT_POMODORO_FOCUS_MS) ==
        AGENT_POMODORO_FOCUS_ENDED);
}

int main(void) {
  idle_shows_a_full_focus();
  toggle_starts_pauses_and_resumes();
  focus_end_waits_for_the_user_to_start_the_break();
  stop_abandons_without_counting();
  stop_skips_a_pending_break();
  tally_counts_completed_focus_and_resets_by_day();
  millisecond_counter_may_wrap();
  if (failures != 0) {
    fprintf(stderr, "%d 处失败\n", failures);
    return EXIT_FAILURE;
  }
  puts("pomodoro: 全部通过");
  return EXIT_SUCCESS;
}
