#pragma once

#include <stdbool.h>
#include <stddef.h>

#include "esp_err.h"

#define AGENT_DISPLAY_MAX_TASKS 3
#define AGENT_DISPLAY_MAX_STATS 3

typedef enum {
  AGENT_DISPLAY_IDLE,
  AGENT_DISPLAY_WORKING,
  AGENT_DISPLAY_INPUT_REQUIRED,
  AGENT_DISPLAY_DONE,
  AGENT_DISPLAY_FAILED,
  AGENT_DISPLAY_OFFLINE,
} agent_display_state_t;

/// The buddy is in exactly one mode at a time, and each mode owns the whole screen. Duty
/// watches the agents, pomodoro times the user, and leisure is the buddy playing on its
/// own after duty has been idle long enough. Agent state keeps updating in all three
/// modes; pomodoro mode just gives it a one-line summary.
typedef enum {
  AGENT_MODE_DUTY,
  AGENT_MODE_POMODORO,
  AGENT_MODE_LEISURE,
} agent_mode_t;

typedef struct {
  const char *title;
  agent_display_state_t state;
  /// Seconds since entering the current state. The device keeps counting on its own,
  /// because the Mac sends nothing while the visible state is unchanged, yet the number
  /// on the card must keep moving.
  int elapsed_s;
  /// Owning project; drawn on the second line when the title is a session name, skipped when empty or the same as the title.
  const char *project;
} agent_display_task_t;

esp_err_t agent_display_init(void);
esp_err_t agent_display_show(agent_display_state_t state, const char *title);
esp_err_t agent_display_show_tasks(agent_display_state_t state,
                                   const char *title,
                                   const agent_display_task_t *tasks,
                                   size_t task_count);
void agent_display_tick(void);

void agent_display_set_mode(agent_mode_t mode);
agent_mode_t agent_display_mode(void);
/// Redraws immediately. After a key press changes the pomodoro, don't wait for the next animation frame.
void agent_display_refresh(void);
/// Whether the agents have nothing going on: main state idle and no task cards. Leisure
/// boredom accumulates on this, not on time since the last message, because long tasks
/// send no messages midway anyway.
bool agent_display_agent_idle(void);

/// Overlay while the link is lost: the buddy closes its eyes and the screen turns gray.
/// The underlying state and task cards are kept: they are the last known facts, just no
/// longer trustworthy.
void agent_display_set_link_lost(bool lost);

/// Blink to identify: the backlight flashes for about a second, visible in any mode. Onboarding uses it to find the box.
void agent_display_identify(void);

/// Alarm at the end of a pomodoro phase: the ring shakes for two seconds, then the whole
/// ring pulses until the user presses a key or leaves the screen. When muted this is the
/// only reminder. The caller must then bring the pomodoro screen to the front.
void agent_display_pomodoro_ended(void);

/// While muted, a MUTE badge stays in the top left: mute is easy to forget, so it must stay visible.
void agent_display_set_muted(bool muted);

/// Screenshot: run-length encodes the current framebuffer and hands it to `write_line`
/// line by line (without newlines). The first line is `SHOT BEGIN 320x240 BACKLIGHT ON|OFF`,
/// each middle line holds several `rgb565:length` runs, and the last is `SHOT END`.
void agent_display_dump(void (*write_line)(const char *line));

/// Records today's stats, which the idle screen rotates through. Takes effect on the next draw.
void agent_display_set_stats(const char *const *lines, size_t count);

/// Sets the build stamps of this firmware and of the Mac side. The device compares them
/// for the user: asking people to read two hashes and compare them isn't reliable, and a
/// mismatch is exactly the signal they need to see.
void agent_display_set_firmware_build(const char *build);
void agent_display_set_daemon_build(const char *build);
