#pragma once

#include <stdbool.h>
#include <stdint.h>

/// Director for leisure mode: tracks boredom, draws skits, and decides when to dim and
/// turn off the backlight. It reads no clock and touches no hardware; every entry point
/// takes a millisecond count from the caller, and wraparound is allowed.

/// How long idle counts as bored, as sleepy, and how long sleepy at night before the backlight goes off.
#define AGENT_LEISURE_BORED_AFTER_MS (5u * 60u * 1000u)
#define AGENT_LEISURE_SLEEPY_AFTER_MS (30u * 60u * 1000u)
#define AGENT_LEISURE_LIGHTS_OUT_AFTER_MS (90u * 60u * 1000u)
/// Frame length of skit animations: 8 fps.
#define AGENT_LEISURE_FRAME_MS 125u

typedef enum {
  /// Standby: the usual breathing, blinking and rotating stats.
  AGENT_LEISURE_ALERT,
  /// Bored: play a short skit every so often.
  AGENT_LEISURE_BORED,
  /// Sleepy: mostly sleeping, with the screen dimmed.
  AGENT_LEISURE_SLEEPY,
} agent_leisure_tier_t;

typedef enum {
  /// Plain idle between skits.
  AGENT_SKIT_NONE,
  AGENT_SKIT_PATROL,
  AGENT_SKIT_BALL,
  AGENT_SKIT_READ,
  AGENT_SKIT_STARS,
  AGENT_SKIT_HIDE,
  AGENT_SKIT_STARTLE,
  AGENT_SKIT_DREAM,
  /// Base look while sleepy: sleeping.
  AGENT_SKIT_SLEEP,
  AGENT_SKIT_COUNT,
} agent_skit_t;

typedef struct {
  agent_leisure_tier_t tier;
  agent_skit_t skit;
  /// How many frames of the current skit have played.
  uint32_t skit_frame;
  /// Sleepy: dim the screen.
  bool dim;
  /// Asleep long enough at night: backlight off.
  bool lights_out;
} agent_leisure_view_t;

void agent_leisure_init(uint32_t seed, uint32_t now_ms);
/// Any activity resets boredom: agent activity, a key press, a running pomodoro.
void agent_leisure_note_activity(uint32_t now_ms);
/// K1 long press: go play right now.
void agent_leisure_force_bored(uint32_t now_ms);
/// Local hour from the Mac's heartbeat; -1 means unknown.
void agent_leisure_set_hour(int hour);
/// Focus sessions or tasks completed today, which decides whether it is tired or bored.
void agent_leisure_set_done_count(unsigned done);
/// Advances the director. Returns true when the level or skit changed.
bool agent_leisure_tick(uint32_t now_ms);
void agent_leisure_view(uint32_t now_ms, agent_leisure_view_t *view);
/// For tests and previews: start a given skit immediately.
void agent_leisure_start_skit(agent_skit_t skit, uint32_t now_ms);

const char *agent_leisure_tier_name(agent_leisure_tier_t tier);
const char *agent_leisure_skit_name(agent_skit_t skit);
