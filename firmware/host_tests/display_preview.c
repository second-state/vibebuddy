// 在 Mac 上把 agent_display.c 的画面渲染成 PPM，用来在烧录前看版式。
// 直接 include 源文件：预览要的是同一份绘制代码，不是它的复制品。
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
    fprintf(stderr, "渲染失败: %s\n", name);
    exit(EXIT_FAILURE);
  }
  write_ppm(path);
}

static void set_ms(uint32_t ms) { stub_tick_count = ms / portTICK_PERIOD_MS; }

int main(int argc, char **argv) {
  if (argc != 2) {
    fprintf(stderr, "用法: %s <输出目录>\n", argv[0]);
    return EXIT_FAILURE;
  }
  const char *directory = argv[1];
  display_ready = true;
  agent_display_set_firmware_build("9b642af 2026-09-14 17:41");
  agent_display_set_daemon_build("9b642af 2026-09-14 17:43");
  const char *stats[] = {"7 DONE", "4 ASKS", "1H23 BUSY"};
  agent_display_set_stats(stats, 3);
  agent_pomodoro_init();

  set_ms(1000);
  agent_display_set_scene(AGENT_SCENE_POMODORO);
  snapshot(directory, "pomodoro_idle");

  agent_display_task_t tasks[] = {
      {"CC:AGENT-BEACON", AGENT_DISPLAY_WORKING, 75},
      {"CX:EROS-TRAINING-INFRA", AGENT_DISPLAY_INPUT_REQUIRED, 900},
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

  agent_display_set_scene(AGENT_SCENE_PET);
  snapshot(directory, "pet_with_badge");

  uint32_t focus_end = 1000 + 25 * 60 * 1000 + 100;
  set_ms(focus_end);
  (void)agent_pomodoro_tick(focus_end);
  agent_display_set_scene(AGENT_SCENE_POMODORO);
  agent_display_show(AGENT_DISPLAY_IDLE, NULL);
  snapshot(directory, "pomodoro_break_pending");

  agent_pomodoro_toggle(focus_end + 30000);
  set_ms(focus_end + 30000 + 2 * 60 * 1000);
  snapshot(directory, "pomodoro_break");

  agent_display_set_link_lost(true);
  snapshot(directory, "pomodoro_no_link");
  return EXIT_SUCCESS;
}
