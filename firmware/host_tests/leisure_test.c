// 休闲导演的主机端测试。由 tools/test-leisure.sh 编译运行。
#include <stdio.h>
#include <stdlib.h>

#include "agent_leisure.h"

static int failures;

#define CHECK(condition)                                                    \
  do {                                                                      \
    if (!(condition)) {                                                     \
      failures++;                                                           \
      fprintf(stderr, "%s:%d: 断言失败: %s\n", __FILE__, __LINE__,          \
              #condition);                                                  \
    }                                                                       \
  } while (0)

#define MINUTES(n) ((uint32_t)(n) * 60u * 1000u)

static agent_leisure_view_t view_at(uint32_t now_ms) {
  agent_leisure_view_t view;
  agent_leisure_view(now_ms, &view);
  return view;
}

/// 把时间推到 now，每 20 ms tick 一次，像主循环那样。有符号比较，允许回绕。
static void advance_to(uint32_t *clock, uint32_t now) {
  while ((int32_t)(now - *clock) > 0) {
    *clock += 20;
    (void)agent_leisure_tick(*clock);
  }
}

static bool is_sleep_family(agent_skit_t skit) {
  return skit == AGENT_SKIT_SLEEP || skit == AGENT_SKIT_DREAM ||
         skit == AGENT_SKIT_STARTLE;
}

static void tiers_follow_idle_time(void) {
  agent_leisure_init(7, 0);
  uint32_t clock = 0;
  advance_to(&clock, MINUTES(4));
  CHECK(view_at(clock).tier == AGENT_LEISURE_ALERT);
  advance_to(&clock, MINUTES(6));
  CHECK(view_at(clock).tier == AGENT_LEISURE_BORED);
  CHECK(!view_at(clock).dim);
  advance_to(&clock, MINUTES(31));
  CHECK(view_at(clock).tier == AGENT_LEISURE_SLEEPY);
  CHECK(view_at(clock).dim);
  CHECK(view_at(clock).skit == AGENT_SKIT_SLEEP ||
        is_sleep_family(view_at(clock).skit));
}

static void activity_resets_everything(void) {
  agent_leisure_init(7, 0);
  uint32_t clock = 0;
  advance_to(&clock, MINUTES(40));
  CHECK(view_at(clock).tier == AGENT_LEISURE_SLEEPY);
  agent_leisure_note_activity(clock);
  (void)agent_leisure_tick(clock);
  agent_leisure_view_t view = view_at(clock);
  CHECK(view.tier == AGENT_LEISURE_ALERT);
  CHECK(view.skit == AGENT_SKIT_NONE);
  CHECK(!view.dim);
  CHECK(!view.lights_out);
}

static void bored_plays_skits_and_never_repeats(void) {
  agent_leisure_init(11, 0);
  uint32_t clock = 0;
  advance_to(&clock, MINUTES(5) + 4000);
  // 进入无聊 3 秒后开第一场。
  CHECK(view_at(clock).skit != AGENT_SKIT_NONE);
  CHECK(view_at(clock).skit != AGENT_SKIT_SLEEP);

  agent_skit_t previous = AGENT_SKIT_NONE;
  bool in_gap = true;
  unsigned played = 0;
  unsigned repeats = 0;
  bool seen[AGENT_SKIT_COUNT] = {false};
  for (unsigned step = 0; step < 4000 && clock < MINUTES(29); step++) {
    advance_to(&clock, clock + 500);
    agent_skit_t current = view_at(clock).skit;
    if (current == AGENT_SKIT_NONE) {
      in_gap = true;
      continue;
    }
    if (!in_gap) {
      continue;  // 同一场还在演
    }
    in_gap = false;
    if (current == previous) {
      repeats++;
    }
    seen[current] = true;
    previous = current;
    played++;
  }
  CHECK(played >= 20);
  CHECK(repeats == 0);
  // 24 分钟里七出都该露过面。
  for (int index = AGENT_SKIT_PATROL; index <= AGENT_SKIT_DREAM; index++) {
    CHECK(seen[index]);
  }
}

static void skit_frames_advance_at_eight_fps(void) {
  agent_leisure_init(3, 0);
  agent_leisure_start_skit(AGENT_SKIT_PATROL, 1000);
  CHECK(view_at(1000).skit_frame == 0);
  CHECK(view_at(1000 + 125 * 8).skit_frame == 8);
  // 12 秒后巡逻结束，回到底色。
  uint32_t clock = 1000;
  advance_to(&clock, 1000 + 12500);
  CHECK(view_at(clock).skit == AGENT_SKIT_NONE);
}

static void sleepy_only_dreams_or_startles(void) {
  agent_leisure_init(5, 0);
  uint32_t clock = 0;
  advance_to(&clock, MINUTES(31));
  for (unsigned step = 0; step < 600; step++) {
    advance_to(&clock, clock + 1000);
    CHECK(is_sleep_family(view_at(clock).skit));
  }
}

static void lights_go_out_only_at_night(void) {
  agent_leisure_init(9, 0);
  agent_leisure_set_hour(14);
  uint32_t clock = 0;
  advance_to(&clock, MINUTES(100));
  CHECK(view_at(clock).tier == AGENT_LEISURE_SLEEPY);
  CHECK(!view_at(clock).lights_out);

  // 22 点还不算夜里，23 点起算，到早上 7 点。
  agent_leisure_set_hour(22);
  CHECK(!view_at(clock).lights_out);
  agent_leisure_set_hour(23);
  CHECK(view_at(clock).lights_out);
  agent_leisure_set_hour(3);
  CHECK(view_at(clock).lights_out);
  agent_leisure_set_hour(7);
  CHECK(!view_at(clock).lights_out);
  agent_leisure_set_hour(23);

  // 关着灯的时候有事，立刻亮。
  agent_leisure_note_activity(clock);
  (void)agent_leisure_tick(clock);
  CHECK(!view_at(clock).lights_out);

  // 夜里但没睡够 90 分钟，不关。
  agent_leisure_init(9, 0);
  agent_leisure_set_hour(1);
  clock = 0;
  advance_to(&clock, MINUTES(60));
  CHECK(view_at(clock).tier == AGENT_LEISURE_SLEEPY);
  CHECK(!view_at(clock).lights_out);
}

static void unknown_hour_is_daytime(void) {
  agent_leisure_init(9, 0);
  uint32_t clock = 0;
  advance_to(&clock, MINUTES(200));
  CHECK(!view_at(clock).lights_out);
}

static void force_bored_starts_playing_now(void) {
  agent_leisure_init(13, 0);
  uint32_t clock = 1000;
  agent_leisure_force_bored(clock);
  (void)agent_leisure_tick(clock);
  CHECK(view_at(clock).tier == AGENT_LEISURE_BORED);
  advance_to(&clock, clock + 4000);
  CHECK(view_at(clock).skit != AGENT_SKIT_NONE);
}

static void nothing_done_means_kicking_the_ball(void) {
  agent_leisure_init(17, 0);
  agent_leisure_set_done_count(0);
  uint32_t clock = 0;
  advance_to(&clock, MINUTES(5) + 4000);
  unsigned ball = 0;
  unsigned played = 0;
  agent_skit_t previous = AGENT_SKIT_NONE;
  while (clock < MINUTES(29)) {
    advance_to(&clock, clock + 500);
    agent_skit_t current = view_at(clock).skit;
    if (current == AGENT_SKIT_NONE || current == previous) {
      continue;
    }
    previous = current;
    played++;
    if (current == AGENT_SKIT_BALL) {
      ball++;
    }
  }
  // 权重 7/22，即便不连演，也该占三成上下。
  CHECK(played >= 20);
  CHECK(ball * 10 >= played * 2);
}

static void millisecond_counter_may_wrap(void) {
  uint32_t start = UINT32_MAX - MINUTES(2);
  agent_leisure_init(19, start);
  uint32_t clock = start;
  advance_to(&clock, start + MINUTES(4));  // 回绕后
  CHECK(view_at(clock).tier == AGENT_LEISURE_ALERT);
  advance_to(&clock, start + MINUTES(6));
  CHECK(view_at(clock).tier == AGENT_LEISURE_BORED);
}

int main(void) {
  tiers_follow_idle_time();
  activity_resets_everything();
  bored_plays_skits_and_never_repeats();
  skit_frames_advance_at_eight_fps();
  sleepy_only_dreams_or_startles();
  lights_go_out_only_at_night();
  unknown_hour_is_daytime();
  force_bored_starts_playing_now();
  nothing_done_means_kicking_the_ball();
  millisecond_counter_may_wrap();
  if (failures != 0) {
    fprintf(stderr, "%d 处失败\n", failures);
    return EXIT_FAILURE;
  }
  puts("leisure: 全部通过");
  return EXIT_SUCCESS;
}
