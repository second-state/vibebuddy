// 番茄钟状态机的主机端测试。由 tools/test-pomodoro.sh 编译运行。
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
  // 晚到 40 ms 的 tick 仍然只报一次结束。
  CHECK(agent_pomodoro_tick(AGENT_POMODORO_FOCUS_MS + 40) ==
        AGENT_POMODORO_FOCUS_ENDED);
  CHECK(agent_pomodoro_tick(AGENT_POMODORO_FOCUS_MS + 80) ==
        AGENT_POMODORO_NOTHING);

  // 休息不自动开始：停在待开始，时间不走。
  agent_pomodoro_view_t view = view_at(AGENT_POMODORO_FOCUS_MS + 60000);
  CHECK(view.phase == AGENT_POMODORO_BREAK);
  CHECK(view.run == AGENT_POMODORO_PENDING);
  CHECK(!agent_pomodoro_is_idle(&view));
  CHECK(view.completed == 1);
  CHECK(view.total_ms == AGENT_POMODORO_BREAK_MS);
  CHECK(view.remaining_ms == AGENT_POMODORO_BREAK_MS);
  CHECK(agent_pomodoro_tick(AGENT_POMODORO_FOCUS_MS + 3600000) ==
        AGENT_POMODORO_NOTHING);

  // 用户按键才开始休息，从按键时刻起算。
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

  // 暂停后再放弃，恢复的旧剩余时间不能泄漏到下一次专注里。
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
  // 跳过休息不抹掉已经完成的那一次专注。
  CHECK(view.completed == 1);
}

static void millisecond_counter_may_wrap(void) {
  agent_pomodoro_init();
  uint32_t start = UINT32_MAX - 1000;
  agent_pomodoro_toggle(start);
  uint32_t wrapped = start + 5000;  // 回绕到 3999
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
  millisecond_counter_may_wrap();
  if (failures != 0) {
    fprintf(stderr, "%d 处失败\n", failures);
    return EXIT_FAILURE;
  }
  puts("pomodoro: 全部通过");
  return EXIT_SUCCESS;
}
