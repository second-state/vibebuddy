#pragma once

#include "esp_err.h"

/// Each of the three keys does one thing, regardless of mode. Short presses fire on
/// release; holding past the threshold fires a long press, and the release then no
/// longer counts as a short press.
typedef enum {
  /// K0 (BOOT key, GPIO0): pomodoro start / pause / resume.
  AGENT_BUTTON_K0_SHORT,
  /// K0 long press: abandon the current phase.
  AGENT_BUTTON_K0_LONG,
  /// K1: switch between duty and pomodoro.
  AGENT_BUTTON_K1_SHORT,
  /// K1 long press: send the buddy off to leisure right now.
  AGENT_BUTTON_K1_LONG,
  /// K2: open the current source, reported to the Mac.
  AGENT_BUTTON_K2_SHORT,
  /// K2 long press: mute toggle.
  AGENT_BUTTON_K2_LONG,
} agent_button_event_t;

typedef void (*agent_button_callback_t)(agent_button_event_t event);

esp_err_t agent_buttons_init(agent_button_callback_t on_event);
void agent_buttons_tick(void);
