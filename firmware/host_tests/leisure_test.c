// Host tests for the leisure director. Built and run by tools/test-leisure.sh.
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

/// Advances time to now, ticking every 20 ms like the main loop. Signed comparison, so wraparound is fine.
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
  // The first skit starts 3 seconds after getting bored.
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
      continue;  // the same skit is still playing
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
  // All seven skits should have appeared within 24 minutes.
  for (int index = AGENT_SKIT_PATROL; index <= AGENT_SKIT_DREAM; index++) {
    CHECK(seen[index]);
  }
}

static void skit_frames_advance_at_eight_fps(void) {
  agent_leisure_init(3, 0);
  agent_leisure_start_skit(AGENT_SKIT_PATROL, 1000);
  CHECK(view_at(1000).skit_frame == 0);
  CHECK(view_at(1000 + 125 * 8).skit_frame == 8);
  // Patrol ends after 12 seconds and returns to the base look.
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

  // 22:00 is not night yet; night runs from 23:00 to 07:00.
  agent_leisure_set_hour(22);
  CHECK(!view_at(clock).lights_out);
  agent_leisure_set_hour(23);
  CHECK(view_at(clock).lights_out);
  agent_leisure_set_hour(3);
  CHECK(view_at(clock).lights_out);
  agent_leisure_set_hour(7);
  CHECK(!view_at(clock).lights_out);
  agent_leisure_set_hour(23);

  // Something happens while the light is off: turn it on immediately.
  agent_leisure_note_activity(clock);
  (void)agent_leisure_tick(clock);
  CHECK(!view_at(clock).lights_out);

  // Night, but not asleep for 90 minutes yet: don't turn off.
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
  // Weight 7/22: even without back-to-back repeats it should be about 30%.
  CHECK(played >= 20);
  CHECK(ball * 10 >= played * 2);
}

static void millisecond_counter_may_wrap(void) {
  uint32_t start = UINT32_MAX - MINUTES(2);
  agent_leisure_init(19, start);
  uint32_t clock = start;
  advance_to(&clock, start + MINUTES(4));  // after wraparound
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
