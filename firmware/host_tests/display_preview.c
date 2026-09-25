// Renders agent_display.c's screens to PPM on the Mac, to check the layout before flashing.
// Includes the source file directly: the preview needs the same drawing code, not a copy of it.
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "freertos/FreeRTOS.h"

TickType_t stub_tick_count;

#include "../main/agent_display.c"

static void write_ppm(const char *path) {
  FILE *file = fopen(path, "wb");
  if (file == NULL) {
    perror(path);
    exit(EXIT_FAILURE);
  }
  fprintf(file, "P6\n%d %d\n255\n", DISPLAY_WIDTH, DISPLAY_HEIGHT);
  for (int index = 0; index < DISPLAY_WIDTH * DISPLAY_HEIGHT; index++) {
    uint16_t pixel = framebuffer[index];
    unsigned char rgb[3] = {
        (unsigned char)(((pixel >> 11) & 0x1f) * 255 / 31),
        (unsigned char)(((pixel >> 5) & 0x3f) * 255 / 63),
        (unsigned char)((pixel & 0x1f) * 255 / 31),
    };
    fwrite(rgb, sizeof(rgb), 1, file);
  }
  fclose(file);
}

static void snapshot(const char *directory, const char *name) {
  char path[512];
  snprintf(path, sizeof(path), "%s/%s.ppm", directory, name);
  if (render_current_state() != ESP_OK) {
    fprintf(stderr, "render failed: %s\n", name);
    exit(EXIT_FAILURE);
  }
  write_ppm(path);
}

static void set_ms(uint32_t ms) { stub_tick_count = ms / portTICK_PERIOD_MS; }

/// Renders every skit frame by frame; a script then stitches them into a GIF.
static void render_skits(const char *directory) {
  static const struct {
    agent_skit_t skit;
    const char *name;
    unsigned frames;
  } SKITS[] = {
      {AGENT_SKIT_PATROL, "patrol", 96},   {AGENT_SKIT_BALL, "ball", 96},
      {AGENT_SKIT_READ, "read", 120},      {AGENT_SKIT_STARS, "stars", 120},
      {AGENT_SKIT_HIDE, "hide", 80},       {AGENT_SKIT_STARTLE, "startle", 80},
      {AGENT_SKIT_DREAM, "dream", 96},     {AGENT_SKIT_SLEEP, "sleep", 48},
      {AGENT_SKIT_NONE, "rest", 48},
  };
  for (unsigned index = 0; index < sizeof(SKITS) / sizeof(SKITS[0]); index++) {
    uint32_t start = 100000;
    agent_leisure_init(1, start);
    agent_leisure_start_skit(SKITS[index].skit, start);
    agent_display_set_mode(AGENT_MODE_DUTY);
    agent_display_set_mode(AGENT_MODE_LEISURE);
    for (unsigned frame = 0; frame < SKITS[index].frames; frame++) {
      set_ms(start + frame * AGENT_LEISURE_FRAME_MS);
      char name[64];
      snprintf(name, sizeof(name), "skit_%s_%03u", SKITS[index].name, frame);
      snapshot(directory, name);
    }
  }
}

int main(int argc, char **argv) {
  if (argc < 2 || argc > 3) {
    fprintf(stderr, "usage: %s <output dir> [leisure]\n", argv[0]);
    return EXIT_FAILURE;
  }
  const char *directory = argv[1];
  display_ready = true;
  agent_display_set_firmware_build("9b642af 2026-09-14 17:41");
  agent_display_set_daemon_build("9b642af 2026-09-14 17:43");
  const char *stats[] = {"7 DONE", "4 ASKS", "1H23 BUSY"};
  agent_display_set_stats(stats, 3);
  agent_pomodoro_init();
  agent_pomodoro_tally_t tally = {20260915, 3, 4500};
  agent_pomodoro_restore_tally(&tally);
  agent_leisure_init(1, 0);
  if (argc == 3 && strcmp(argv[2], "leisure") == 0) {
    render_skits(directory);
    return EXIT_SUCCESS;
  }

  set_ms(1000);
  agent_display_set_mode(AGENT_MODE_POMODORO);
  snapshot(directory, "pomodoro_idle");

  agent_display_task_t tasks[] = {
      {"CC:POMODORO TIMER FEATURE", AGENT_DISPLAY_WORKING, 75, "AGENT-BEACON"},
      {"CX:EROS-TRAINING-INFRA", AGENT_DISPLAY_INPUT_REQUIRED, 900,
       "EROS-TRAINING-INFRA"},
  };
  agent_display_show_tasks(AGENT_DISPLAY_INPUT_REQUIRED, "CC:AGENT-BEACON",
                           tasks, 2);
  agent_pomodoro_toggle(1000);
  set_ms(1000 + 6 * 60 * 1000 + 39 * 1000);
  snapshot(directory, "pomodoro_focus");

  agent_pomodoro_toggle(1000 + 6 * 60 * 1000 + 39 * 1000);
  animation_frame = 1;
  snapshot(directory, "pomodoro_paused_blink");
  animation_frame = 0;
  snapshot(directory, "pomodoro_paused");
  agent_pomodoro_toggle(1000 + 6 * 60 * 1000 + 39 * 1000);

  agent_display_set_mode(AGENT_MODE_DUTY);
  snapshot(directory, "pet_with_badge");

  uint32_t focus_end = 1000 + 25 * 60 * 1000 + 100;
  set_ms(focus_end);
  (void)agent_pomodoro_tick(focus_end);
  agent_display_pomodoro_ended();
  agent_display_set_mode(AGENT_MODE_POMODORO);
  agent_display_show(AGENT_DISPLAY_IDLE, NULL);

  // The alarm's first three seconds frame by frame: 20 shake frames of 100 ms each,
  // then a 500 ms pulse per beat; stitched into a GIF at 10 fps, one beat is 5 frames.
  for (unsigned frame = 0; frame < 30; frame++) {
    if (frame < RING_ALARM_SHAKE_FRAMES) {
      animation_frame = frame;
      ring_alarm_shake_frames = RING_ALARM_SHAKE_FRAMES - frame;
    } else {
      animation_frame = (frame - RING_ALARM_SHAKE_FRAMES) / 5;
      ring_alarm_shake_frames = 0;
    }
    char name[64];
    snprintf(name, sizeof(name), "pomodoro_alarm_%03u", frame);
    snapshot(directory, name);
  }
  animation_frame = 1;
  ring_alarm_shake_frames = RING_ALARM_SHAKE_FRAMES;
  snapshot(directory, "pomodoro_alarm_shake");
  ring_alarm_shake_frames = 0;
  snapshot(directory, "pomodoro_alarm_pulse_dim");
  animation_frame = 0;
  snapshot(directory, "pomodoro_alarm_pulse");
  // The waiting-to-start look needs checking too, so dismiss the alarm first.
  ring_alarm = false;
  snapshot(directory, "pomodoro_break_pending");

  agent_pomodoro_toggle(focus_end + 30000);
  set_ms(focus_end + 30000 + 2 * 60 * 1000);
  snapshot(directory, "pomodoro_break");

  agent_display_set_link_lost(true);
  snapshot(directory, "pomodoro_no_link");
  agent_display_set_link_lost(false);
  agent_display_set_muted(true);
  agent_display_set_mode(AGENT_MODE_DUTY);
  snapshot(directory, "pet_muted");
  return EXIT_SUCCESS;
}
