#include "agent_display.h"

#include <ctype.h>
#include <math.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "agent_pomodoro.h"
#include "driver/gpio.h"
#include "driver/i2c_master.h"
#include "esp_check.h"
#include "esp_lcd_panel_io.h"
#include "esp_lcd_panel_ops.h"
#include "esp_lcd_panel_vendor.h"
#include "freertos/FreeRTOS.h"
#include "freertos/semphr.h"
#include "freertos/task.h"

#define DISPLAY_WIDTH 320
#define DISPLAY_HEIGHT 240

#define LCD_NUM_CS GPIO_NUM_1
#define LCD_NUM_DC GPIO_NUM_2
#define LCD_NUM_RD GPIO_NUM_41
#define LCD_NUM_WR GPIO_NUM_42

#define GPIO_LCD_D0 GPIO_NUM_40
#define GPIO_LCD_D1 GPIO_NUM_39
#define GPIO_LCD_D2 GPIO_NUM_38
#define GPIO_LCD_D3 GPIO_NUM_12
#define GPIO_LCD_D4 GPIO_NUM_11
#define GPIO_LCD_D5 GPIO_NUM_10
#define GPIO_LCD_D6 GPIO_NUM_9
#define GPIO_LCD_D7 GPIO_NUM_46

#define I2C_SDA_GPIO GPIO_NUM_48
#define I2C_SCL_GPIO GPIO_NUM_45
#define XL9555_ADDRESS 0x20
#define XL9555_OUTPUT_PORT0 0x02
#define XL9555_CONFIG_PORT0 0x06
#define XL9555_LCD_BACKLIGHT_MASK 0x80

#define COLOR_BACKGROUND 0x0841
#define COLOR_MUTED 0x8410
#define COLOR_TEXT 0xffff
#define COLOR_READY 0x2dff
#define COLOR_WORKING 0xfd20
#define COLOR_INPUT 0xffe0
#define COLOR_DONE 0x07e0
#define COLOR_FAILED 0xf800
#define COLOR_PET 0x3c9f
#define COLOR_PET_HIGHLIGHT 0x7e5f
/// 番茄钟：专注是番茄红，休息是绿色。
#define COLOR_FOCUS 0xfa8a
#define COLOR_BREAK 0x4ecc

#define TITLE_BYTES 64
/// 构建标识：git 描述加上编译时刻。
#define BUILD_BYTES 48

/// 空闲时每隔 IDLE_MOOD_PERIOD 帧做一个小动作，持续 IDLE_MOOD_FRAMES 帧。
#define IDLE_MOOD_PERIOD 40
#define IDLE_MOOD_FRAMES 8
/// 空闲时标题与战绩的轮播间隔（帧）。空闲动画每 500 ms 一帧。
#define IDLE_ROTATE_FRAMES 6

/// 番茄钟画面：左边是刻度圆环与倒计时，右边是阶段、按键提示与 Agent 摘要。
/// 圆环仿 Focus To-Do：一圈刻度，走过的部分染成阶段色，一根更长的指针停在
/// 当前位置；空闲时指针停在 12 点。
#define RING_CENTER_X 118
#define RING_CENTER_Y 122
#define RING_TICKS 60
#define RING_TICK_INNER 70
#define RING_TICK_OUTER 79
#define RING_HAND_INNER 64
#define RING_HAND_OUTER 86
#define PANEL_X 208
#define TAU 6.2831853f

static const char *TAG = "agent_display";

static const uint8_t LETTER_GLYPHS[26][7] = {
    {0x0e, 0x11, 0x11, 0x1f, 0x11, 0x11, 0x11},
    {0x1e, 0x11, 0x11, 0x1e, 0x11, 0x11, 0x1e},
    {0x0e, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0e},
    {0x1e, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1e},
    {0x1f, 0x10, 0x10, 0x1e, 0x10, 0x10, 0x1f},
    {0x1f, 0x10, 0x10, 0x1e, 0x10, 0x10, 0x10},
    {0x0e, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0f},
    {0x11, 0x11, 0x11, 0x1f, 0x11, 0x11, 0x11},
    {0x1f, 0x04, 0x04, 0x04, 0x04, 0x04, 0x1f},
    {0x07, 0x02, 0x02, 0x02, 0x12, 0x12, 0x0c},
    {0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11},
    {0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1f},
    {0x11, 0x1b, 0x15, 0x15, 0x11, 0x11, 0x11},
    {0x11, 0x19, 0x15, 0x13, 0x11, 0x11, 0x11},
    {0x0e, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0e},
    {0x1e, 0x11, 0x11, 0x1e, 0x10, 0x10, 0x10},
    {0x0e, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0d},
    {0x1e, 0x11, 0x11, 0x1e, 0x14, 0x12, 0x11},
    {0x0f, 0x10, 0x10, 0x0e, 0x01, 0x01, 0x1e},
    {0x1f, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04},
    {0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0e},
    {0x11, 0x11, 0x11, 0x11, 0x11, 0x0a, 0x04},
    {0x11, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0a},
    {0x11, 0x11, 0x0a, 0x04, 0x0a, 0x11, 0x11},
    {0x11, 0x11, 0x0a, 0x04, 0x04, 0x04, 0x04},
    {0x1f, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1f},
};

static const uint8_t DIGIT_GLYPHS[10][7] = {
    {0x0e, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0e},
    {0x04, 0x0c, 0x04, 0x04, 0x04, 0x04, 0x0e},
    {0x0e, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1f},
    {0x1e, 0x01, 0x01, 0x0e, 0x01, 0x01, 0x1e},
    {0x02, 0x06, 0x0a, 0x12, 0x1f, 0x02, 0x02},
    {0x1f, 0x10, 0x10, 0x1e, 0x01, 0x01, 0x1e},
    {0x0e, 0x10, 0x10, 0x1e, 0x11, 0x11, 0x0e},
    {0x1f, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08},
    {0x0e, 0x11, 0x11, 0x0e, 0x11, 0x11, 0x0e},
    {0x0e, 0x11, 0x11, 0x0f, 0x01, 0x01, 0x0e},
};

static uint16_t framebuffer[DISPLAY_WIDTH * DISPLAY_HEIGHT]
    __attribute__((aligned(4)));
static esp_lcd_panel_handle_t panel_handle;
static i2c_master_dev_handle_t xl9555_handle;
static SemaphoreHandle_t transfer_done;
static bool display_ready;
static agent_display_state_t current_state = AGENT_DISPLAY_IDLE;
static bool link_lost = false;
static agent_scene_t current_scene = AGENT_SCENE_PET;
static char current_title[TITLE_BYTES];
static struct {
  char title[TITLE_BYTES];
  agent_display_state_t state;
  /// 收到这张卡时它已经持续了多久，以及收到的时刻。可见状态不变时 Mac
  /// 端不会再发消息，卡片上的数字却必须继续走，所以由设备自己接着算。
  int elapsed_base;
  TickType_t received_tick;
} current_tasks[AGENT_DISPLAY_MAX_TASKS];
static size_t current_task_count;
static char current_stats[AGENT_DISPLAY_MAX_STATS][TITLE_BYTES];
static size_t current_stat_count;
static char firmware_build[BUILD_BYTES];
static char daemon_build[BUILD_BYTES];
static uint32_t animation_frame;
static TickType_t next_animation_at;

static bool on_color_transfer_done(esp_lcd_panel_io_handle_t panel_io,
                                   esp_lcd_panel_io_event_data_t *event_data,
                                   void *user_context) {
  (void)panel_io;
  (void)event_data;
  BaseType_t task_woken = pdFALSE;
  xSemaphoreGiveFromISR((SemaphoreHandle_t)user_context, &task_woken);
  return task_woken == pdTRUE;
}

static esp_err_t xl9555_read(uint8_t reg, uint8_t *value) {
  return i2c_master_transmit_receive(xl9555_handle, &reg, 1, value, 1,
                                     pdMS_TO_TICKS(100));
}

static esp_err_t xl9555_write(uint8_t reg, uint8_t value) {
  uint8_t command[] = {reg, value};
  return i2c_master_transmit(xl9555_handle, command, sizeof(command),
                             pdMS_TO_TICKS(100));
}

static esp_err_t set_backlight(bool enabled) {
  uint8_t output;
  ESP_RETURN_ON_ERROR(xl9555_read(XL9555_OUTPUT_PORT0, &output), TAG,
                      "读取 XL9555 output 失败");
  if (enabled) {
    output |= XL9555_LCD_BACKLIGHT_MASK;
  } else {
    output &= (uint8_t)~XL9555_LCD_BACKLIGHT_MASK;
  }
  return xl9555_write(XL9555_OUTPUT_PORT0, output);
}

static void fill_rect(int x, int y, int width, int height, uint16_t color) {
  int x_start = x < 0 ? 0 : x;
  int y_start = y < 0 ? 0 : y;
  int x_end = x + width > DISPLAY_WIDTH ? DISPLAY_WIDTH : x + width;
  int y_end = y + height > DISPLAY_HEIGHT ? DISPLAY_HEIGHT : y + height;

  for (int row = y_start; row < y_end; row++) {
    for (int column = x_start; column < x_end; column++) {
      framebuffer[row * DISPLAY_WIDTH + column] = color;
    }
  }
}

static void draw_line(int x0, int y0, int x1, int y1, uint16_t color) {
  int delta_x = abs(x1 - x0);
  int step_x = x0 < x1 ? 1 : -1;
  int delta_y = -abs(y1 - y0);
  int step_y = y0 < y1 ? 1 : -1;
  int error = delta_x + delta_y;

  while (true) {
    fill_rect(x0 - 1, y0 - 1, 3, 3, color);
    if (x0 == x1 && y0 == y1) {
      break;
    }
    int doubled_error = 2 * error;
    if (doubled_error >= delta_y) {
      error += delta_y;
      x0 += step_x;
    }
    if (doubled_error <= delta_x) {
      error += delta_x;
      y0 += step_y;
    }
  }
}

static uint8_t glyph_row(char character, int row) {
  unsigned char uppercase = (unsigned char)toupper((unsigned char)character);
  if (uppercase >= 'A' && uppercase <= 'Z') {
    return LETTER_GLYPHS[uppercase - 'A'][row];
  }
  if (uppercase >= '0' && uppercase <= '9') {
    return DIGIT_GLYPHS[uppercase - '0'][row];
  }
  if (uppercase == '-') {
    return row == 3 ? 0x1f : 0;
  }
  if (uppercase == '.') {
    return row == 6 ? 0x04 : 0;
  }
  if (uppercase == ':') {
    return row == 2 || row == 5 ? 0x04 : 0;
  }
  if (uppercase == '/') {
    return (uint8_t)(1U << (row < 5 ? 4 - row : 0));
  }
  if (uppercase == '!') {
    return row < 5 || row == 6 ? 0x04 : 0;
  }
  if (uppercase == '?') {
    static const uint8_t question[7] = {0x0e, 0x11, 0x01, 0x02,
                                        0x04, 0x00, 0x04};
    return question[row];
  }
  if (uppercase == '>') {
    static const uint8_t chevron[7] = {0x10, 0x08, 0x04, 0x02,
                                       0x04, 0x08, 0x10};
    return chevron[row];
  }
  if (uppercase == ' ' || uppercase == '_') {
    return uppercase == '_' && row == 6 ? 0x1f : 0;
  }
  return row == 0 || row == 3 ? 0x0e : (row == 1 || row == 2 ? 0x11 : 0x04);
}

static void draw_text(int x, int y, const char *text, int scale, uint16_t color,
                      size_t max_characters) {
  for (size_t index = 0; text[index] != '\0' && index < max_characters;
       index++) {
    for (int row = 0; row < 7; row++) {
      uint8_t bits = glyph_row(text[index], row);
      for (int column = 0; column < 5; column++) {
        if ((bits & (1U << (4 - column))) != 0) {
          fill_rect(x + (int)index * 6 * scale + column * scale,
                    y + row * scale, scale, scale, color);
        }
      }
    }
  }
}

static void draw_text_centered(int y, const char *text, int scale,
                               uint16_t color) {
  size_t max_characters = DISPLAY_WIDTH / (6 * scale);
  size_t length = strnlen(text, max_characters);
  int width = length == 0 ? 0 : (int)(length * 6 - 1) * scale;
  draw_text((DISPLAY_WIDTH - width) / 2, y, text, scale, color, max_characters);
}

typedef enum {
  IDLE_MOOD_NONE,
  IDLE_MOOD_NAP,
  IDLE_MOOD_LOOK,
  IDLE_MOOD_STRETCH,
} idle_mood_t;

/// 小动作已经进行了几帧；负数表示当前没有小动作。
static int idle_mood_phase(uint32_t frame) {
  return (int)(frame % IDLE_MOOD_PERIOD) - (IDLE_MOOD_PERIOD - IDLE_MOOD_FRAMES);
}

/// 空闲时轮流做三个小动作。呼吸和眨眼之外还得有点别的，否则一台一直亮着
/// 的设备看上去更像卡住了而不是在待命。
static idle_mood_t idle_mood(uint32_t frame) {
  static const idle_mood_t cycle[] = {IDLE_MOOD_NAP, IDLE_MOOD_LOOK,
                                      IDLE_MOOD_STRETCH};
  if (idle_mood_phase(frame) < 0) {
    return IDLE_MOOD_NONE;
  }
  return cycle[(frame / IDLE_MOOD_PERIOD) % 3];
}

static void draw_pet_face(agent_display_state_t state, uint32_t frame,
                          int x_offset, int y_offset, uint16_t color) {
  int face_y = 79 + y_offset;
  if (state == AGENT_DISPLAY_WORKING) {
    draw_text(139 + x_offset, face_y + 5, ">", 2, color, 1);
    int dot_count = (int)(frame % 3) + 1;
    for (int index = 0; index < dot_count; index++) {
      fill_rect(163 + x_offset + index * 8, face_y + 17, 5, 3, color);
    }
  } else if (state == AGENT_DISPLAY_INPUT_REQUIRED) {
    draw_text(141 + x_offset, face_y + 5, "!?", 2, color, 2);
  } else if (state == AGENT_DISPLAY_DONE) {
    draw_line(140 + x_offset, face_y + 12, 147 + x_offset, face_y + 6, color);
    draw_line(147 + x_offset, face_y + 6, 154 + x_offset, face_y + 12, color);
    draw_line(166 + x_offset, face_y + 12, 173 + x_offset, face_y + 6, color);
    draw_line(173 + x_offset, face_y + 6, 180 + x_offset, face_y + 12, color);
    draw_line(151 + x_offset, face_y + 19, 160 + x_offset, face_y + 23, color);
    draw_line(160 + x_offset, face_y + 23, 169 + x_offset, face_y + 19, color);
  } else if (state == AGENT_DISPLAY_FAILED) {
    draw_line(140 + x_offset, face_y + 7, 153 + x_offset, face_y + 18, color);
    draw_line(153 + x_offset, face_y + 7, 140 + x_offset, face_y + 18, color);
    draw_line(167 + x_offset, face_y + 7, 180 + x_offset, face_y + 18, color);
    draw_line(180 + x_offset, face_y + 7, 167 + x_offset, face_y + 18, color);
    draw_line(153 + x_offset, face_y + 25, 167 + x_offset, face_y + 25, color);
  } else if (state == AGENT_DISPLAY_OFFLINE) {
    // 闭眼与平直的嘴：睡着，而不是出错。
    fill_rect(143 + x_offset, face_y + 14, 8, 3, color);
    fill_rect(169 + x_offset, face_y + 14, 8, 3, color);
    draw_line(154 + x_offset, face_y + 25, 166 + x_offset, face_y + 25, color);
  } else {
    idle_mood_t mood = idle_mood(frame);
    if (mood == IDLE_MOOD_NAP) {
      // 打盹：闭眼加一个飘起来的 Z。和失联的闭眼靠颜色与这个 Z 区分。
      fill_rect(143 + x_offset, face_y + 14, 8, 3, color);
      fill_rect(169 + x_offset, face_y + 14, 8, 3, color);
      draw_line(154 + x_offset, face_y + 25, 166 + x_offset, face_y + 25,
                color);
      draw_text(172 + x_offset, 44 + y_offset - idle_mood_phase(frame), "Z", 2,
                color, 1);
      return;
    }
    // 左顾右盼：只平移眼睛，看上去像在打量房间。
    int gaze = mood == IDLE_MOOD_LOOK ? (frame % 4 < 2 ? -3 : 3) : 0;
    bool blinking = frame % 8 == 7;
    fill_rect(143 + x_offset + gaze, face_y + (blinking ? 14 : 8), 8,
              blinking ? 3 : 10, color);
    fill_rect(169 + x_offset + gaze, face_y + (blinking ? 14 : 8), 8,
              blinking ? 3 : 10, color);
    draw_line(154 + x_offset, face_y + 24, 160 + x_offset, face_y + 27, color);
    draw_line(160 + x_offset, face_y + 27, 166 + x_offset, face_y + 24, color);
  }
}

static void draw_beaconling(agent_display_state_t state, uint32_t frame,
                            int x_offset, uint16_t color) {
  int y_offset = 0;
  if (state == AGENT_DISPLAY_WORKING) {
    y_offset = frame % 2 == 0 ? 0 : 2;
  } else if (state == AGENT_DISPLAY_DONE) {
    y_offset = frame % 2 == 0 ? -5 : 0;
  } else if (state == AGENT_DISPLAY_FAILED) {
    y_offset = 3;
  } else if (state == AGENT_DISPLAY_IDLE && frame % 8 == 0) {
    y_offset = 1;
  }

  // 伸懒腰时只拉长天线、身体不动，才像伸展而不是整只跳一下。
  int antenna = state == AGENT_DISPLAY_IDLE &&
                        idle_mood(frame) == IDLE_MOOD_STRETCH
                    ? 5
                    : 0;
  fill_rect(157 + x_offset, 48 + y_offset - antenna, 6, 13 + antenna,
            COLOR_PET_HIGHLIGHT);
  fill_rect(153 + x_offset, 44 + y_offset - antenna, 14, 10, color);

  fill_rect(113 + x_offset, 65 + y_offset, 94, 52, COLOR_PET);
  fill_rect(121 + x_offset, 59 + y_offset, 78, 64, COLOR_PET);
  fill_rect(105 + x_offset, 78 + y_offset, 12, 28, COLOR_PET_HIGHLIGHT);
  fill_rect(203 + x_offset, 78 + y_offset, 12, 28, COLOR_PET_HIGHLIGHT);
  fill_rect(128 + x_offset, 75 + y_offset, 64, 40, COLOR_BACKGROUND);
  fill_rect(132 + x_offset, 79 + y_offset, 56, 32, 0x10a4);
  draw_pet_face(state, frame, x_offset, y_offset, color);

  fill_rect(139 + x_offset, 121 + y_offset, 42, 25, COLOR_PET);
  fill_rect(126 + x_offset, 125 + y_offset, 13, 17, COLOR_PET_HIGHLIGHT);
  fill_rect(181 + x_offset, 125 + y_offset, 13, 17, COLOR_PET_HIGHLIGHT);
  fill_rect(143 + x_offset, 145 + y_offset, 13, 8, COLOR_PET_HIGHLIGHT);
  fill_rect(164 + x_offset, 145 + y_offset, 13, 8, COLOR_PET_HIGHLIGHT);
}

static uint16_t state_color(agent_display_state_t state) {
  if (link_lost) {
    // 失联期间所有颜色转灰：状态可能已经过时，不该继续用鲜艳色宣称它成立。
    return COLOR_MUTED;
  }
  if (state == AGENT_DISPLAY_WORKING) {
    return COLOR_WORKING;
  }
  if (state == AGENT_DISPLAY_INPUT_REQUIRED) {
    return COLOR_INPUT;
  }
  if (state == AGENT_DISPLAY_DONE) {
    return COLOR_DONE;
  }
  if (state == AGENT_DISPLAY_FAILED) {
    return COLOR_FAILED;
  }
  return COLOR_READY;
}

static const char *short_state_label(agent_display_state_t state) {
  if (state == AGENT_DISPLAY_INPUT_REQUIRED) {
    return "ASK";
  }
  if (state == AGENT_DISPLAY_DONE) {
    return "DONE";
  }
  if (state == AGENT_DISPLAY_FAILED) {
    return "FAIL";
  }
  return "RUN";
}

/// 收到卡片时的秒数加上设备自己走过的时间。
static int task_elapsed_seconds(size_t index) {
  TickType_t ticks = xTaskGetTickCount() - current_tasks[index].received_tick;
  return current_tasks[index].elapsed_base + (int)(pdTICKS_TO_MS(ticks) / 1000);
}

/// 卡片右下角只有三格宽，超过一小时就只报小时。
static void format_elapsed(char *out, size_t size, int seconds) {
  if (seconds < 0) {
    seconds = 0;
  }
  if (seconds < 60) {
    snprintf(out, size, "%dS", seconds);
  } else if (seconds < 3600) {
    snprintf(out, size, "%dM", seconds / 60);
  } else {
    snprintf(out, size, "%dH", seconds / 3600);
  }
}

static void draw_task_cards(void) {
  for (size_t index = 0; index < current_task_count; index++) {
    int x = 8 + (int)index * 4;
    int y = 50 + (int)index * 38;
    int width = 188 - (int)index * 4;
    uint16_t card_color = index == 0 ? 0x18c6 : 0x1083;
    uint16_t color = state_color(current_tasks[index].state);
    fill_rect(x, y, width, 32, COLOR_MUTED);
    fill_rect(x + 2, y + 2, width - 4, 28, card_color);
    fill_rect(x + 2, y + 2, 4, 28, color);
    draw_text(x + 12, y + 5, current_tasks[index].title, 1, COLOR_TEXT, 26);
    draw_text(x + 12, y + 17, short_state_label(current_tasks[index].state), 1,
              color, 4);

    char elapsed[8];
    format_elapsed(elapsed, sizeof(elapsed), task_elapsed_seconds(index));
    int elapsed_width = (int)strlen(elapsed) * 6 - 1;
    // 等待确认时把时长也点亮：这一栏回答的正是「等了多久」。
    uint16_t elapsed_color =
        current_tasks[index].state == AGENT_DISPLAY_INPUT_REQUIRED ? color
                                                                   : COLOR_MUTED;
    draw_text(x + width - 8 - elapsed_width, y + 17, elapsed, 1, elapsed_color,
              sizeof(elapsed));
  }
}

/// 页脚显示两侧的构建标识：本机固件，以及心跳捎来的 Mac 端。
///
/// 只显示，不判断。固件要插 USB、停 daemon 才能烧，daemon 改一行就重启，
/// 两边大部分时间本来就不在同一个 commit 上；把「不一致」当成告警，几天内
/// 就会被彻底无视。真正会出事的是协议能力不匹配，而那不是 commit 能回答的。
///
/// 两行左对齐到同一列——逐字比对靠的是对齐，不是颜色。
static void draw_build_footer(void) {
  char firmware_line[BUILD_BYTES + 8];
  char daemon_line[BUILD_BYTES + 8];
  snprintf(firmware_line, sizeof(firmware_line), "FW     %s",
           firmware_build[0] == '\0' ? "?" : firmware_build);
  snprintf(daemon_line, sizeof(daemon_line), "DAEMON %s",
           daemon_build[0] == '\0' ? "?" : daemon_build);

  size_t firmware_length = strlen(firmware_line);
  size_t daemon_length = strlen(daemon_line);
  size_t longest = firmware_length > daemon_length ? firmware_length : daemon_length;
  int x = (DISPLAY_WIDTH - (int)(longest * 6 - 1)) / 2;
  if (x < 2) {
    x = 2;
  }
  draw_text(x, 216, firmware_line, 1, COLOR_MUTED, sizeof(firmware_line));
  draw_text(x, 228, daemon_line, 1, COLOR_MUTED, sizeof(daemon_line));
}

/// 空闲时在标题与战绩之间轮播。空闲屏出现得最频繁，只写一句固定的话太浪费。
static void draw_idle_line(void) {
  const char *lines[1 + AGENT_DISPLAY_MAX_STATS];
  size_t count = 0;
  bool has_title = current_title[0] != '\0';
  if (has_title) {
    lines[count++] = current_title;
  }
  for (size_t index = 0; index < current_stat_count; index++) {
    lines[count++] = current_stats[index];
  }
  if (count == 0) {
    draw_text_centered(195, "YOUR AGENT PET", 2, COLOR_MUTED);
    return;
  }
  size_t slot = (animation_frame / IDLE_ROTATE_FRAMES) % count;
  draw_text_centered(195, lines[slot], 2,
                     has_title && slot == 0 ? COLOR_TEXT : COLOR_MUTED);
}

static esp_err_t present(void) {
  while (xSemaphoreTake(transfer_done, 0) == pdTRUE) {
  }
  ESP_RETURN_ON_ERROR(esp_lcd_panel_draw_bitmap(panel_handle, 0, 0,
                                                DISPLAY_WIDTH, DISPLAY_HEIGHT,
                                                framebuffer),
                      TAG, "提交 LCD framebuffer 失败");
  if (xSemaphoreTake(transfer_done, pdMS_TO_TICKS(1000)) != pdTRUE) {
    return ESP_ERR_TIMEOUT;
  }
  return ESP_OK;
}

static const char *state_label(agent_display_state_t state) {
  if (link_lost) {
    return "NO LINK";
  }
  if (state == AGENT_DISPLAY_WORKING) {
    return "WORKING";
  }
  if (state == AGENT_DISPLAY_INPUT_REQUIRED) {
    return "INPUT REQUIRED";
  }
  if (state == AGENT_DISPLAY_DONE) {
    return "DONE";
  }
  if (state == AGENT_DISPLAY_FAILED) {
    return "FAILED";
  }
  return "READY";
}

/// 与主循环用同一种毫秒计数，番茄钟的截止时刻才对得上。
static uint32_t clock_ms(void) {
  return xTaskGetTickCount() * (uint32_t)portTICK_PERIOD_MS;
}

static uint16_t phase_color(const agent_pomodoro_view_t *view) {
  if (agent_pomodoro_is_idle(view)) {
    return COLOR_READY;
  }
  return view->phase == AGENT_POMODORO_BREAK ? COLOR_BREAK : COLOR_FOCUS;
}

/// 从圆心向外画一段径向线。角度从 12 点起顺时针。
static void draw_radial(float angle, int inner, int outer, int thickness,
                        uint16_t color) {
  float dx = sinf(angle);
  float dy = -cosf(angle);
  for (int radius = inner; radius <= outer; radius++) {
    int x = RING_CENTER_X + (int)lroundf(dx * (float)radius);
    int y = RING_CENTER_Y + (int)lroundf(dy * (float)radius);
    fill_rect(x - thickness / 2, y - thickness / 2, thickness, thickness,
              color);
  }
}

static void draw_pomodoro_ring(const agent_pomodoro_view_t *view) {
  uint16_t color = phase_color(view);
  uint32_t elapsed = view->total_ms - view->remaining_ms;
  float sweep = TAU * (float)elapsed / (float)view->total_ms;
  for (int index = 0; index < RING_TICKS; index++) {
    float angle = TAU * (float)index / RING_TICKS;
    bool passed = view->run != AGENT_POMODORO_PENDING && angle <= sweep;
    draw_radial(angle, RING_TICK_INNER, RING_TICK_OUTER, 2,
                passed ? color : COLOR_MUTED);
  }
  draw_radial(sweep, RING_HAND_INNER, RING_HAND_OUTER, 3, color);
}

/// 向上取整到秒：刚开始显示 25:00，走到最后一毫秒仍是 00:01。
static void format_countdown(char *out, size_t size, uint32_t remaining_ms) {
  uint32_t seconds = (remaining_ms + 999u) / 1000u;
  snprintf(out, size, "%02u:%02u", (unsigned)(seconds / 60u) % 100u,
           (unsigned)(seconds % 60u));
}

/// 番茄钟场景里 Agent 只剩右下角几行：它仍然回答“现在最需要我注意什么”，
/// 语音也照常播，只是画面让给了倒计时。
static void draw_agent_summary(const char *label, uint16_t status_color) {
  draw_text(PANEL_X, 176, "AGENT", 1, COLOR_MUTED, 18);
  draw_text(PANEL_X, 188, label, 1, status_color, 18);
  if (current_task_count > 0) {
    draw_text(PANEL_X, 200, current_tasks[0].title, 1,
              link_lost ? COLOR_MUTED : COLOR_TEXT, 18);
  }
}

static void draw_pomodoro_scene(const char *label, uint16_t status_color) {
  agent_pomodoro_view_t view;
  agent_pomodoro_view(clock_ms(), &view);
  uint16_t color = phase_color(&view);
  bool paused = view.run == AGENT_POMODORO_PAUSED;
  draw_pomodoro_ring(&view);

  // 暂停时数字闪烁：停表的老规矩。
  if (!paused || animation_frame % 2 == 0) {
    char countdown[8];
    format_countdown(countdown, sizeof(countdown), view.remaining_ms);
    draw_text(RING_CENTER_X - 58, RING_CENTER_Y - 14, countdown, 4,
              COLOR_TEXT, 5);
  }

  // 空闲写 READY；专注刚结束、休息还没开始时写 BREAK 配 05:00，
  // 和空闲区分开：这一屏在等的是开始休息，不是开始专注。
  const char *phase_label = agent_pomodoro_is_idle(&view)          ? "READY"
                            : view.phase == AGENT_POMODORO_BREAK ? "BREAK"
                                                                 : "FOCUS";
  draw_text(PANEL_X, 44, phase_label, 2, color, 9);
  if (view.run == AGENT_POMODORO_PENDING) {
    draw_text(PANEL_X, 66, "K0 START", 1, COLOR_MUTED, 18);
    if (!agent_pomodoro_is_idle(&view)) {
      draw_text(PANEL_X, 78, "HOLD K0 SKIP", 1, COLOR_MUTED, 18);
    }
  } else {
    draw_text(PANEL_X, 66, paused ? "K0 RESUME" : "K0 PAUSE", 1, COLOR_MUTED,
              18);
    draw_text(PANEL_X, 78, "HOLD K0 STOP", 1, COLOR_MUTED, 18);
  }
  if (paused) {
    draw_text(PANEL_X, 98, "PAUSED", 2, color, 9);
  }

  // 完成一次专注记一格，开机后累计。
  unsigned shown = view.completed > 8 ? 8 : view.completed;
  for (unsigned index = 0; index < shown; index++) {
    fill_rect(PANEL_X + (int)index * 12, 128, 8, 8, COLOR_FOCUS);
  }
  if (view.completed > 8) {
    char more[8];
    snprintf(more, sizeof(more), "+%u", (view.completed - 8) % 1000u);
    draw_text(PANEL_X + 96, 128, more, 1, COLOR_FOCUS, sizeof(more));
  }

  draw_agent_summary(label, status_color);
}

/// 小灯灵场景右上角的番茄钟小徽章：切回来看 Agent 时，倒计时不该消失。
static void draw_pomodoro_badge(void) {
  agent_pomodoro_view_t view;
  agent_pomodoro_view(clock_ms(), &view);
  if (agent_pomodoro_is_idle(&view) ||
      (view.run == AGENT_POMODORO_PAUSED && animation_frame % 2 == 1)) {
    return;
  }
  char countdown[8];
  format_countdown(countdown, sizeof(countdown), view.remaining_ms);
  char badge[12];
  snprintf(badge, sizeof(badge), "%c %s",
           view.phase == AGENT_POMODORO_FOCUS ? 'F' : 'B', countdown);
  draw_text(DISPLAY_WIDTH - 8 - (7 * 6 - 1), 15, badge, 1,
            phase_color(&view), sizeof(badge));
}

static void draw_pet_scene(const char *label, uint16_t status_color) {
  if (current_task_count > 0) {
    draw_task_cards();
  }
  draw_beaconling(link_lost ? AGENT_DISPLAY_OFFLINE : current_state,
                  animation_frame, current_task_count > 0 ? 96 : 0,
                  status_color);
  draw_text_centered(162, label,
                     current_state == AGENT_DISPLAY_INPUT_REQUIRED ? 2 : 3,
                     status_color);
  if (current_task_count > 0) {
    draw_text_centered(195, "LATEST ON TOP", 1, COLOR_MUTED);
  } else if (current_state == AGENT_DISPLAY_IDLE && !link_lost) {
    // 失联时不轮播：会动的画面看上去像还活着，正好与 NO LINK 相反。
    draw_idle_line();
  } else if (current_title[0] != '\0') {
    draw_text_centered(195, current_title, 2, COLOR_TEXT);
  }
  draw_pomodoro_badge();
}

static esp_err_t render_current_state(void) {
  const char *label = state_label(current_state);
  uint16_t status_color = state_color(current_state);

  fill_rect(0, 0, DISPLAY_WIDTH, DISPLAY_HEIGHT, COLOR_BACKGROUND);
  fill_rect(0, 0, DISPLAY_WIDTH, 4, status_color);
  draw_text_centered(12, "AgentBeacon", 2, COLOR_TEXT);
  if (current_scene == AGENT_SCENE_POMODORO) {
    draw_pomodoro_scene(label, status_color);
  } else {
    draw_pet_scene(label, status_color);
  }
  draw_build_footer();
  return present();
}

static TickType_t animation_period(agent_display_state_t state) {
  if (current_scene == AGENT_SCENE_POMODORO) {
    // 倒计时每秒变一次；暂停时的闪烁要每半秒一帧。
    return pdMS_TO_TICKS(500);
  }
  if (state == AGENT_DISPLAY_WORKING) {
    return pdMS_TO_TICKS(250);
  }
  if (state == AGENT_DISPLAY_DONE) {
    return pdMS_TO_TICKS(200);
  }
  if (state == AGENT_DISPLAY_INPUT_REQUIRED) {
    return pdMS_TO_TICKS(350);
  }
  if (state == AGENT_DISPLAY_FAILED) {
    return pdMS_TO_TICKS(700);
  }
  return pdMS_TO_TICKS(500);
}

esp_err_t agent_display_show(agent_display_state_t state, const char *title) {
  return agent_display_show_tasks(state, title, NULL, 0);
}

esp_err_t agent_display_show_tasks(agent_display_state_t state,
                                   const char *title,
                                   const agent_display_task_t *tasks,
                                   size_t task_count) {
  if (!display_ready) {
    return ESP_ERR_INVALID_STATE;
  }

  current_state = state;
  animation_frame = 0;
  if (title == NULL) {
    current_title[0] = '\0';
  } else {
    strncpy(current_title, title, sizeof(current_title) - 1);
    current_title[sizeof(current_title) - 1] = '\0';
  }
  current_task_count = task_count > AGENT_DISPLAY_MAX_TASKS
                           ? AGENT_DISPLAY_MAX_TASKS
                           : task_count;
  for (size_t index = 0; index < current_task_count; index++) {
    const char *task_title =
        tasks[index].title == NULL ? "CODEX" : tasks[index].title;
    strncpy(current_tasks[index].title, task_title,
            sizeof(current_tasks[index].title) - 1);
    current_tasks[index].title[sizeof(current_tasks[index].title) - 1] = '\0';
    current_tasks[index].state = tasks[index].state;
    current_tasks[index].elapsed_base = tasks[index].elapsed_s;
    current_tasks[index].received_tick = xTaskGetTickCount();
  }
  next_animation_at = xTaskGetTickCount() + animation_period(current_state);
  return render_current_state();
}

void agent_display_set_stats(const char *const *lines, size_t count) {
  current_stat_count =
      count > AGENT_DISPLAY_MAX_STATS ? AGENT_DISPLAY_MAX_STATS : count;
  for (size_t index = 0; index < current_stat_count; index++) {
    const char *line = lines[index] == NULL ? "" : lines[index];
    strncpy(current_stats[index], line, sizeof(current_stats[index]) - 1);
    current_stats[index][sizeof(current_stats[index]) - 1] = '\0';
  }
}

/// 心跳每 5 秒一次，标识没变就不能重画。
static void set_build(char *slot, const char *build) {
  const char *value = build == NULL ? "" : build;
  if (strncmp(slot, value, BUILD_BYTES - 1) == 0) {
    return;
  }
  strncpy(slot, value, BUILD_BYTES - 1);
  slot[BUILD_BYTES - 1] = '\0';
  if (display_ready) {
    (void)render_current_state();
  }
}

void agent_display_set_firmware_build(const char *build) {
  set_build(firmware_build, build);
}

void agent_display_set_daemon_build(const char *build) {
  set_build(daemon_build, build);
}

void agent_display_set_link_lost(bool lost) {
  if (link_lost == lost) {
    return;
  }
  link_lost = lost;
  (void)render_current_state();
}

void agent_display_set_scene(agent_scene_t scene) {
  if (current_scene == scene) {
    return;
  }
  current_scene = scene;
  animation_frame = 0;
  next_animation_at = xTaskGetTickCount() + animation_period(current_state);
  if (display_ready) {
    (void)render_current_state();
  }
}

agent_scene_t agent_display_scene(void) { return current_scene; }

void agent_display_refresh(void) {
  if (display_ready) {
    (void)render_current_state();
  }
}

void agent_display_tick(void) {
  if (!display_ready ||
      (int32_t)(xTaskGetTickCount() - next_animation_at) < 0) {
    return;
  }

  animation_frame++;
  next_animation_at = xTaskGetTickCount() + animation_period(current_state);
  if (render_current_state() != ESP_OK) {
    next_animation_at = xTaskGetTickCount() + pdMS_TO_TICKS(1000);
  }
}

esp_err_t agent_display_init(void) {
  i2c_master_bus_config_t i2c_config = {
      .i2c_port = I2C_NUM_0,
      .sda_io_num = I2C_SDA_GPIO,
      .scl_io_num = I2C_SCL_GPIO,
      .clk_source = I2C_CLK_SRC_DEFAULT,
      .glitch_ignore_cnt = 7,
      .flags.enable_internal_pullup = true,
  };
  i2c_master_bus_handle_t i2c_bus;
  ESP_RETURN_ON_ERROR(i2c_new_master_bus(&i2c_config, &i2c_bus), TAG,
                      "初始化 I2C 失败");

  i2c_device_config_t xl9555_config = {
      .dev_addr_length = I2C_ADDR_BIT_LEN_7,
      .device_address = XL9555_ADDRESS,
      .scl_speed_hz = 400000,
  };
  ESP_RETURN_ON_ERROR(
      i2c_master_bus_add_device(i2c_bus, &xl9555_config, &xl9555_handle), TAG,
      "添加 XL9555 失败");

  uint8_t direction;
  ESP_RETURN_ON_ERROR(xl9555_read(XL9555_CONFIG_PORT0, &direction), TAG,
                      "探测 XL9555 失败");
  direction &= (uint8_t)~XL9555_LCD_BACKLIGHT_MASK;
  ESP_RETURN_ON_ERROR(xl9555_write(XL9555_CONFIG_PORT0, direction), TAG,
                      "配置 LCD 背光方向失败");
  ESP_RETURN_ON_ERROR(set_backlight(false), TAG, "关闭 LCD 背光失败");

  gpio_config_t read_pin_config = {
      .pin_bit_mask = 1ULL << LCD_NUM_RD,
      .mode = GPIO_MODE_INPUT_OUTPUT,
      .pull_up_en = GPIO_PULLUP_ENABLE,
  };
  ESP_RETURN_ON_ERROR(gpio_config(&read_pin_config), TAG, "配置 LCD RD 失败");
  ESP_RETURN_ON_ERROR(gpio_set_level(LCD_NUM_RD, 1), TAG, "拉高 LCD RD 失败");

  esp_lcd_i80_bus_handle_t i80_bus;
  esp_lcd_i80_bus_config_t bus_config = {
      .dc_gpio_num = LCD_NUM_DC,
      .wr_gpio_num = LCD_NUM_WR,
      .clk_src = LCD_CLK_SRC_DEFAULT,
      .data_gpio_nums = {GPIO_LCD_D0, GPIO_LCD_D1, GPIO_LCD_D2, GPIO_LCD_D3,
                         GPIO_LCD_D4, GPIO_LCD_D5, GPIO_LCD_D6, GPIO_LCD_D7},
      .bus_width = 8,
      .max_transfer_bytes = sizeof(framebuffer),
      .psram_trans_align = 64,
      .sram_trans_align = 4,
  };
  ESP_RETURN_ON_ERROR(esp_lcd_new_i80_bus(&bus_config, &i80_bus), TAG,
                      "初始化 LCD i80 bus 失败");

  transfer_done = xSemaphoreCreateBinary();
  if (transfer_done == NULL) {
    return ESP_ERR_NO_MEM;
  }

  esp_lcd_panel_io_i80_config_t io_config = {
      .cs_gpio_num = LCD_NUM_CS,
      .pclk_hz = 10 * 1000 * 1000,
      .trans_queue_depth = 2,
      .on_color_trans_done = on_color_transfer_done,
      .user_ctx = transfer_done,
      .lcd_cmd_bits = 8,
      .lcd_param_bits = 8,
      .dc_levels =
          {
              .dc_idle_level = 0,
              .dc_cmd_level = 0,
              .dc_dummy_level = 0,
              .dc_data_level = 1,
          },
      .flags.swap_color_bytes = 1,
  };
  esp_lcd_panel_io_handle_t panel_io;
  ESP_RETURN_ON_ERROR(esp_lcd_new_panel_io_i80(i80_bus, &io_config, &panel_io),
                      TAG, "初始化 LCD panel IO 失败");

  esp_lcd_panel_dev_config_t panel_config = {
      .reset_gpio_num = GPIO_NUM_NC,
      .rgb_ele_order = LCD_RGB_ELEMENT_ORDER_RGB,
      .bits_per_pixel = 16,
  };
  ESP_RETURN_ON_ERROR(
      esp_lcd_new_panel_st7789(panel_io, &panel_config, &panel_handle), TAG,
      "初始化 ST7789 失败");
  ESP_RETURN_ON_ERROR(esp_lcd_panel_reset(panel_handle), TAG,
                      "复位 ST7789 失败");
  ESP_RETURN_ON_ERROR(esp_lcd_panel_init(panel_handle), TAG,
                      "配置 ST7789 失败");
  ESP_RETURN_ON_ERROR(esp_lcd_panel_invert_color(panel_handle, true), TAG,
                      "设置 LCD 颜色反转失败");
  ESP_RETURN_ON_ERROR(esp_lcd_panel_set_gap(panel_handle, 0, 0), TAG,
                      "设置 LCD offset 失败");

  uint8_t memory_access = 0x00;
  uint8_t pixel_format = 0x65;
  ESP_RETURN_ON_ERROR(
      esp_lcd_panel_io_tx_param(panel_io, 0x36, &memory_access, 1), TAG,
      "设置 LCD memory access 失败");
  ESP_RETURN_ON_ERROR(
      esp_lcd_panel_io_tx_param(panel_io, 0x3a, &pixel_format, 1), TAG,
      "设置 LCD pixel format 失败");
  ESP_RETURN_ON_ERROR(esp_lcd_panel_swap_xy(panel_handle, true), TAG,
                      "设置 LCD 方向失败");
  ESP_RETURN_ON_ERROR(esp_lcd_panel_mirror(panel_handle, true, false), TAG,
                      "设置 LCD 镜像失败");
  ESP_RETURN_ON_ERROR(esp_lcd_panel_disp_on_off(panel_handle, true), TAG,
                      "打开 LCD panel 失败");

  display_ready = true;
  ESP_RETURN_ON_ERROR(agent_display_show(AGENT_DISPLAY_IDLE, NULL), TAG,
                      "绘制初始页面失败");
  ESP_RETURN_ON_ERROR(set_backlight(true), TAG, "打开 LCD 背光失败");
  return ESP_OK;
}
