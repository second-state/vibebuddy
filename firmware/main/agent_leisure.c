#include "agent_leisure.h"

// 只依赖标准头：这个导演要能在 Mac 上直接编译跑测试（tools/test-leisure.sh）。

// 实机验收时把所有时限压缩，几分钟内走完无聊、困倦、关背光、被唤醒。
// 正式固件不定义它。
#ifndef AGENT_TIME_SCALE
#define AGENT_TIME_SCALE 1u
#endif
#define BORED_AFTER_MS (AGENT_LEISURE_BORED_AFTER_MS / AGENT_TIME_SCALE)
#define SLEEPY_AFTER_MS (AGENT_LEISURE_SLEEPY_AFTER_MS / AGENT_TIME_SCALE)
#define LIGHTS_OUT_AFTER_MS \
  (AGENT_LEISURE_LIGHTS_OUT_AFTER_MS / AGENT_TIME_SCALE)

/// 进入一个档位后多久开第一场；之后的间隔按档位随机。
#define FIRST_SKIT_DELAY_MS (3000u)
#define BORED_GAP_MIN_MS (20000u / AGENT_TIME_SCALE)
#define BORED_GAP_MAX_MS (40000u / AGENT_TIME_SCALE)
#define SLEEPY_GAP_MIN_MS (120000u / AGENT_TIME_SCALE)
#define SLEEPY_GAP_MAX_MS (300000u / AGENT_TIME_SCALE)

/// 各剧目时长；0 表示一直演到档位变化。
static const uint32_t SKIT_LENGTH_MS[AGENT_SKIT_COUNT] = {
    [AGENT_SKIT_NONE] = 0,      [AGENT_SKIT_PATROL] = 12000,
    [AGENT_SKIT_BALL] = 12000,  [AGENT_SKIT_READ] = 15000,
    [AGENT_SKIT_STARS] = 15000, [AGENT_SKIT_HIDE] = 10000,
    [AGENT_SKIT_STARTLE] = 10000, [AGENT_SKIT_DREAM] = 12000,
    [AGENT_SKIT_SLEEP] = 0,
};

static uint32_t rng_state;
static uint32_t idle_since_ms;
static int hour = -1;
static unsigned done_count;
static agent_leisure_tier_t tier;
static agent_skit_t skit;
static agent_skit_t last_skit;
static uint32_t skit_started_ms;
static uint32_t next_skit_at_ms;

static uint32_t rng_next(void) {
  // xorshift32：够随机，也够小。
  uint32_t x = rng_state;
  x ^= x << 13;
  x ^= x >> 17;
  x ^= x << 5;
  rng_state = x;
  return x;
}

static uint32_t rng_between(uint32_t low, uint32_t high) {
  return low + rng_next() % (high - low + 1u);
}

/// 夜里：23 点到早上 7 点。不知道几点就当白天，宁可亮着也不要在下午关灯。
static bool is_night(void) { return hour >= 23 || (hour >= 0 && hour < 7); }

static agent_leisure_tier_t tier_for(uint32_t now_ms) {
  uint32_t idle = now_ms - idle_since_ms;
  if (idle >= SLEEPY_AFTER_MS) {
    return AGENT_LEISURE_SLEEPY;
  }
  if (idle >= BORED_AFTER_MS) {
    return AGENT_LEISURE_BORED;
  }
  return AGENT_LEISURE_ALERT;
}

static agent_skit_t base_skit(agent_leisure_tier_t which) {
  return which == AGENT_LEISURE_SLEEPY ? AGENT_SKIT_SLEEP : AGENT_SKIT_NONE;
}

static uint32_t gap_for(agent_leisure_tier_t which) {
  if (which == AGENT_LEISURE_SLEEPY) {
    return rng_between(SLEEPY_GAP_MIN_MS, SLEEPY_GAP_MAX_MS);
  }
  return rng_between(BORED_GAP_MIN_MS, BORED_GAP_MAX_MS);
}

/// 剧目权重。夜里多睡少玩；今天一件没做就无聊地踢球，做得多就累得梦多。
static void skit_weights(agent_leisure_tier_t which, unsigned *weights) {
  for (int index = 0; index < AGENT_SKIT_COUNT; index++) {
    weights[index] = 0;
  }
  if (which == AGENT_LEISURE_SLEEPY) {
    weights[AGENT_SKIT_DREAM] = 3;
    weights[AGENT_SKIT_STARTLE] = done_count >= 5 ? 0 : 2;
    if (done_count >= 5) {
      weights[AGENT_SKIT_DREAM] = 4;
    }
    return;
  }
  if (is_night()) {
    weights[AGENT_SKIT_PATROL] = 1;
    weights[AGENT_SKIT_BALL] = 1;
    weights[AGENT_SKIT_READ] = 2;
    weights[AGENT_SKIT_STARS] = 4;
    weights[AGENT_SKIT_HIDE] = 1;
    weights[AGENT_SKIT_STARTLE] = 2;
    weights[AGENT_SKIT_DREAM] = 3;
  } else {
    weights[AGENT_SKIT_PATROL] = 3;
    weights[AGENT_SKIT_BALL] = 3;
    weights[AGENT_SKIT_READ] = 3;
    weights[AGENT_SKIT_STARS] = 1;
    weights[AGENT_SKIT_HIDE] = 3;
    weights[AGENT_SKIT_STARTLE] = 1;
    weights[AGENT_SKIT_DREAM] = 1;
  }
  if (done_count == 0) {
    weights[AGENT_SKIT_BALL] += 4;
  } else if (done_count >= 5) {
    weights[AGENT_SKIT_DREAM] += 2;
    weights[AGENT_SKIT_STARTLE] += 1;
  }
}

static agent_skit_t pick_skit(agent_leisure_tier_t which) {
  unsigned weights[AGENT_SKIT_COUNT];
  skit_weights(which, weights);
  // 不连着演同一出；只剩一出可选时才允许重复。
  unsigned total = 0;
  unsigned total_without_last = 0;
  for (int index = 0; index < AGENT_SKIT_COUNT; index++) {
    total += weights[index];
    if (index != (int)last_skit) {
      total_without_last += weights[index];
    }
  }
  if (total_without_last > 0) {
    weights[last_skit] = 0;
    total = total_without_last;
  }
  if (total == 0) {
    return base_skit(which);
  }
  uint32_t roll = rng_next() % total;
  for (int index = 0; index < AGENT_SKIT_COUNT; index++) {
    if (roll < weights[index]) {
      return (agent_skit_t)index;
    }
    roll -= weights[index];
  }
  return base_skit(which);
}

static void enter_tier(agent_leisure_tier_t which, uint32_t now_ms) {
  tier = which;
  skit = base_skit(which);
  skit_started_ms = now_ms;
  next_skit_at_ms = now_ms + FIRST_SKIT_DELAY_MS;
}

void agent_leisure_init(uint32_t seed, uint32_t now_ms) {
  rng_state = seed == 0 ? 0x9e3779b9u : seed;
  idle_since_ms = now_ms;
  hour = -1;
  done_count = 0;
  last_skit = AGENT_SKIT_NONE;
  enter_tier(AGENT_LEISURE_ALERT, now_ms);
}

void agent_leisure_note_activity(uint32_t now_ms) { idle_since_ms = now_ms; }

void agent_leisure_force_bored(uint32_t now_ms) {
  idle_since_ms = now_ms - BORED_AFTER_MS;
}

void agent_leisure_set_hour(int value) { hour = value; }

void agent_leisure_set_done_count(unsigned done) { done_count = done; }

bool agent_leisure_tick(uint32_t now_ms) {
  bool changed = false;
  agent_leisure_tier_t next_tier = tier_for(now_ms);
  if (next_tier != tier) {
    enter_tier(next_tier, now_ms);
    changed = true;
  }
  if (tier == AGENT_LEISURE_ALERT) {
    return changed;
  }
  agent_skit_t base = base_skit(tier);
  if (skit != base &&
      (int32_t)(now_ms - skit_started_ms) >= (int32_t)SKIT_LENGTH_MS[skit]) {
    last_skit = skit;
    skit = base;
    skit_started_ms = now_ms;
    next_skit_at_ms = now_ms + gap_for(tier);
    changed = true;
  }
  if (skit == base && (int32_t)(now_ms - next_skit_at_ms) >= 0) {
    skit = pick_skit(tier);
    skit_started_ms = now_ms;
    changed = true;
  }
  return changed;
}

void agent_leisure_view(uint32_t now_ms, agent_leisure_view_t *view) {
  view->tier = tier;
  view->skit = skit;
  view->skit_frame = (now_ms - skit_started_ms) / AGENT_LEISURE_FRAME_MS;
  view->dim = tier == AGENT_LEISURE_SLEEPY;
  view->lights_out = tier == AGENT_LEISURE_SLEEPY && is_night() &&
                     now_ms - idle_since_ms >= LIGHTS_OUT_AFTER_MS;
}

void agent_leisure_start_skit(agent_skit_t which, uint32_t now_ms) {
  agent_leisure_tier_t needed =
      which == AGENT_SKIT_SLEEP ? AGENT_LEISURE_SLEEPY : AGENT_LEISURE_BORED;
  idle_since_ms = now_ms - (needed == AGENT_LEISURE_SLEEPY ? SLEEPY_AFTER_MS
                                                            : BORED_AFTER_MS);
  enter_tier(needed, now_ms);
  skit = which;
  skit_started_ms = now_ms;
  next_skit_at_ms = now_ms + SLEEPY_GAP_MAX_MS;
}

const char *agent_leisure_tier_name(agent_leisure_tier_t which) {
  switch (which) {
    case AGENT_LEISURE_BORED:
      return "BORED";
    case AGENT_LEISURE_SLEEPY:
      return "SLEEPY";
    default:
      return "ALERT";
  }
}

const char *agent_leisure_skit_name(agent_skit_t which) {
  static const char *const NAMES[AGENT_SKIT_COUNT] = {
      "NONE", "PATROL", "BALL",  "READ",  "STARS",
      "HIDE", "STARTLE", "DREAM", "SLEEP",
  };
  return which < AGENT_SKIT_COUNT ? NAMES[which] : "NONE";
}
