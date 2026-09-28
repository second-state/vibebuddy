#include "agent_display.h"

#include <ctype.h>
#include <math.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "agent_leisure.h"
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
/// Pomodoro: focus is tomato red, break is green.
#define COLOR_FOCUS 0xfa8a
#define COLOR_BREAK 0x4ecc

#define TITLE_BYTES 64
/// Build stamp: git description plus build time.
#define BUILD_BYTES 48

/// While idle, do a small move every IDLE_MOOD_PERIOD frames, lasting IDLE_MOOD_FRAMES frames.
#define IDLE_MOOD_PERIOD 40
#define IDLE_MOOD_FRAMES 8
/// Interval (frames) for rotating the title and stats while idle. The idle animation runs one frame per 500 ms.
#define IDLE_ROTATE_FRAMES 6

/// Pomodoro screen: a tick ring and countdown on the left; phase, key hints and agent summary on the right.
/// The ring mimics Focus To-Do: a ring of ticks, the elapsed part tinted in the phase color, and a longer hand
/// at the current position; while idle the hand rests at 12 o'clock.
#define RING_CENTER_X 118
#define RING_CENTER_Y 122
#define RING_TICKS 60
#define RING_TICK_INNER 70
#define RING_TICK_OUTER 79
#define RING_HAND_INNER 64
#define RING_HAND_OUTER 86
/// Alarm at a phase end: for the first two seconds the ring shakes side to side, switching every frame; then the
/// whole ring pulses frame by frame until the user presses a key. When muted this is the only reminder.
#define RING_ALARM_SHAKE_FRAMES 20
#define RING_ALARM_SHAKE_FRAME_MS 100
#define RING_ALARM_SHAKE_PX 3
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
static agent_mode_t current_mode = AGENT_MODE_DUTY;
static char current_title[TITLE_BYTES];
static struct {
  char title[TITLE_BYTES];
  char project[TITLE_BYTES];
  agent_display_state_t state;
  /// How long the card had lasted when received, and when it was received. The Mac sends nothing while the
  /// visible state is unchanged, yet the number on the card must keep moving, so the device keeps counting.
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
/// Whether the backlight is on. Leisure mode turns it off after sleeping long at night; anything at all turns it back on.
static bool backlight_on;
/// Blink to identify: the backlight flashes until this time; 0 means not flashing.
static TickType_t identify_until;
static TickType_t identify_next_toggle;
/// Whether to dim the whole frame before committing it: the sleepy look.
static bool render_dim;
static bool muted;
/// The alarm is ringing: the phase ended and the user hasn't acted yet. Records the phase at the end; stops as
/// soon as the view changes (start, abandon, skip) or the screen is switched away.
static bool ring_alarm;
static agent_pomodoro_phase_t ring_alarm_phase;
/// Shake frames left; after shaking, switch to pulsing.
static unsigned ring_alarm_shake_frames;

static uint32_t clock_ms(void);
static void format_tally(char *out, size_t size, unsigned completed,
                         uint32_t focus_s);

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
                      "failed to read XL9555 output");
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

/// How many frames the small move has run; negative means no small move right now.
static int idle_mood_phase(uint32_t frame) {
  return (int)(frame % IDLE_MOOD_PERIOD) - (IDLE_MOOD_PERIOD - IDLE_MOOD_FRAMES);
}

/// Cycle through three small moves while idle. Breathing and blinking alone aren't enough: a device that is always
/// lit would look stuck rather than on standby.
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
    // Closed eyes and a flat mouth: asleep, not an error.
    fill_rect(143 + x_offset, face_y + 14, 8, 3, color);
    fill_rect(169 + x_offset, face_y + 14, 8, 3, color);
    draw_line(154 + x_offset, face_y + 25, 166 + x_offset, face_y + 25, color);
  } else {
    idle_mood_t mood = idle_mood(frame);
    if (mood == IDLE_MOOD_NAP) {
      // Dozing: closed eyes plus a floating Z. The color and this Z tell it apart from the link-lost closed eyes.
      fill_rect(143 + x_offset, face_y + 14, 8, 3, color);
      fill_rect(169 + x_offset, face_y + 14, 8, 3, color);
      draw_line(154 + x_offset, face_y + 25, 166 + x_offset, face_y + 25,
                color);
      draw_text(172 + x_offset, 44 + y_offset - idle_mood_phase(frame), "Z", 2,
                color, 1);
      return;
    }
    // Looking around: only the eyes move sideways, as if sizing up the room.
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

/// Antenna, body, arms and the screen on the face. x and y are offsets from the duty position; raised arms use
/// positive values. The face and legs are drawn separately, since each has other poses too.
static void draw_pet_body(int x, int y, int antenna, uint16_t knob_color,
                          int left_arm, int right_arm) {
  fill_rect(157 + x, 48 + y - antenna, 6, 13 + antenna, COLOR_PET_HIGHLIGHT);
  fill_rect(153 + x, 44 + y - antenna, 14, 10, knob_color);

  fill_rect(113 + x, 65 + y, 94, 52, COLOR_PET);
  fill_rect(121 + x, 59 + y, 78, 64, COLOR_PET);
  fill_rect(105 + x, 78 + y - left_arm, 12, 28, COLOR_PET_HIGHLIGHT);
  fill_rect(203 + x, 78 + y - right_arm, 12, 28, COLOR_PET_HIGHLIGHT);
  fill_rect(128 + x, 75 + y, 64, 40, COLOR_BACKGROUND);
  fill_rect(132 + x, 79 + y, 56, 32, 0x10a4);
}

/// Lower body and both feet; raised feet use positive values.
static void draw_pet_legs(int x, int y, int left_foot, int right_foot) {
  fill_rect(139 + x, 121 + y, 42, 25, COLOR_PET);
  fill_rect(126 + x, 125 + y - left_foot, 13, 17, COLOR_PET_HIGHLIGHT);
  fill_rect(181 + x, 125 + y - right_foot, 13, 17, COLOR_PET_HIGHLIGHT);
  fill_rect(143 + x, 145 + y - left_foot, 13, 8, COLOR_PET_HIGHLIGHT);
  fill_rect(164 + x, 145 + y - right_foot, 13, 8, COLOR_PET_HIGHLIGHT);
}

static void draw_buddy(agent_display_state_t state, uint32_t frame,
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

  // When stretching, only lengthen the antenna and keep the body still, so it reads as a stretch rather than a hop.
  int antenna = state == AGENT_DISPLAY_IDLE &&
                        idle_mood(frame) == IDLE_MOOD_STRETCH
                    ? 5
                    : 0;
  draw_pet_body(x_offset, y_offset, antenna, color, 0, 0);
  draw_pet_face(state, frame, x_offset, y_offset, color);
  draw_pet_legs(x_offset, y_offset, 0, 0);
}

static uint16_t state_color(agent_display_state_t state) {
  if (link_lost) {
    // Everything turns gray while the link is lost: the state may be stale and shouldn't keep claiming it in bright colors.
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

/// Seconds at the time the card was received, plus the time the device has counted since.
static int task_elapsed_seconds(size_t index) {
  TickType_t ticks = xTaskGetTickCount() - current_tasks[index].received_tick;
  return current_tasks[index].elapsed_base + (int)(pdTICKS_TO_MS(ticks) / 1000);
}

/// The card's bottom-right corner is only three cells wide; past an hour, show hours only.
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
    // When the first line is a session name, the project name moves to the second line; if the title is the project name, don't repeat it.
    const char *project = current_tasks[index].project;
    if (project[0] != '\0' && strstr(current_tasks[index].title, project) == NULL) {
      draw_text(x + 42, y + 17, project, 1, COLOR_MUTED, 16);
    }

    char elapsed[8];
    format_elapsed(elapsed, sizeof(elapsed), task_elapsed_seconds(index));
    int elapsed_width = (int)strlen(elapsed) * 6 - 1;
    // While waiting for input, light up the duration too: this column answers exactly "how long has it waited".
    uint16_t elapsed_color =
        current_tasks[index].state == AGENT_DISPLAY_INPUT_REQUIRED ? color
                                                                   : COLOR_MUTED;
    draw_text(x + width - 8 - elapsed_width, y + 17, elapsed, 1, elapsed_color,
              sizeof(elapsed));
  }
}

/// The footer shows the build stamps of both sides: this firmware, and the Mac side carried by the heartbeat.
///
/// Display only, no judgement. Flashing firmware means plugging in USB and stopping the daemon, while the daemon
/// restarts after a one-line change, so most of the time the two sides aren't on the same commit; treating a mismatch as a
/// warning would get it ignored within days. What actually breaks is a protocol capability mismatch, which a commit can't answer.
///
/// Both lines are left-aligned to the same column: verbatim comparison relies on alignment, not color.
static void draw_build_footer(void) {
  char firmware_line[BUILD_BYTES + 8];
  char daemon_line[BUILD_BYTES + 8];
  snprintf(firmware_line, sizeof(firmware_line), "FW     %s",
           firmware_build[0] == '\0' ? "?" : firmware_build);
  snprintf(daemon_line, sizeof(daemon_line), "APP    %s",
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

/// While idle, rotate between the title and stats. The idle screen shows up most often, so a single fixed sentence is a waste.
/// The pomodoro's daily record is kept by the device itself and goes into the rotation too.
static void draw_idle_line(void) {
  const char *lines[2 + AGENT_DISPLAY_MAX_STATS];
  size_t count = 0;
  bool has_title = current_title[0] != '\0';
  if (has_title) {
    lines[count++] = current_title;
  }
  for (size_t index = 0; index < current_stat_count; index++) {
    lines[count++] = current_stats[index];
  }
  agent_pomodoro_view_t pomodoro;
  agent_pomodoro_view(clock_ms(), &pomodoro);
  static char tally_line[24];
  if (pomodoro.completed > 0) {
    format_tally(tally_line, sizeof(tally_line), pomodoro.completed,
                 pomodoro.focus_s);
    lines[count++] = tally_line;
  }
  if (count == 0) {
    draw_text_centered(195, "YOUR VIBE BUDDY", 2, COLOR_MUTED);
    return;
  }
  size_t slot = (animation_frame / IDLE_ROTATE_FRAMES) % count;
  draw_text_centered(195, lines[slot], 2,
                     has_title && slot == 0 ? COLOR_TEXT : COLOR_MUTED);
}

static esp_err_t present(void) {
  if (render_dim) {
    // Halve each channel: shift the whole RGB565 value right by one, then mask off the low bits spilling into neighboring channels.
    for (size_t index = 0; index < DISPLAY_WIDTH * DISPLAY_HEIGHT; index++) {
      framebuffer[index] = (uint16_t)((framebuffer[index] >> 1) & 0x7bef);
    }
  }
  while (xSemaphoreTake(transfer_done, 0) == pdTRUE) {
  }
  ESP_RETURN_ON_ERROR(esp_lcd_panel_draw_bitmap(panel_handle, 0, 0,
                                                DISPLAY_WIDTH, DISPLAY_HEIGHT,
                                                framebuffer),
                      TAG, "failed to submit LCD framebuffer");
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

/// Uses the same millisecond count as the main loop so the pomodoro deadline lines up.
static uint32_t clock_ms(void) {
  return xTaskGetTickCount() * (uint32_t)portTICK_PERIOD_MS;
}

static uint16_t phase_color(const agent_pomodoro_view_t *view) {
  if (agent_pomodoro_is_idle(view)) {
    return COLOR_READY;
  }
  return view->phase == AGENT_POMODORO_BREAK ? COLOR_BREAK : COLOR_FOCUS;
}

/// Draws a radial segment outward from the center. Angles run clockwise from 12 o'clock; shift is the whole ring's
/// horizontal offset, nonzero only while the alarm shakes.
static void draw_radial(float angle, int inner, int outer, int thickness,
                        uint16_t color, int shift) {
  float dx = sinf(angle);
  float dy = -cosf(angle);
  for (int radius = inner; radius <= outer; radius++) {
    int x = RING_CENTER_X + shift + (int)lroundf(dx * (float)radius);
    int y = RING_CENTER_Y + (int)lroundf(dy * (float)radius);
    fill_rect(x - thickness / 2, y - thickness / 2, thickness, thickness,
              color);
  }
}

/// Halves each RGB565 component: the dim beat of the pulse.
static uint16_t half_bright(uint16_t color) { return (color >> 1) & 0x7bef; }

/// Whether the alarm is still ringing. After the end, any change of view means the user acted: starting the next
/// phase makes it running, abandoning or skipping changes the phase.
static bool ring_alarm_active(const agent_pomodoro_view_t *view) {
  if (ring_alarm && (view->run != AGENT_POMODORO_PENDING ||
                     view->phase != ring_alarm_phase)) {
    ring_alarm = false;
    ring_alarm_shake_frames = 0;
  }
  return ring_alarm;
}

static bool ring_alarm_shaking(void) {
  return ring_alarm && ring_alarm_shake_frames > 0;
}

/// The whole ring's horizontal offset while shaking: switches side every frame.
static int ring_alarm_shift(void) {
  if (!ring_alarm_shaking()) {
    return 0;
  }
  return animation_frame % 2 == 0 ? RING_ALARM_SHAKE_PX : -RING_ALARM_SHAKE_PX;
}

static void draw_pomodoro_ring(const agent_pomodoro_view_t *view, bool alarm) {
  uint16_t color = phase_color(view);
  uint32_t elapsed = view->total_ms - view->remaining_ms;
  float sweep = TAU * (float)elapsed / (float)view->total_ms;
  int shift = ring_alarm_shift();
  // Alarm: the whole ring lights up in the next phase's color. After shaking, alternate a bright and a dim frame like a
  // heartbeat, not a hard flash between gray and bright.
  if (alarm && !ring_alarm_shaking() && animation_frame % 2 == 1) {
    color = half_bright(color);
  }
  for (int index = 0; index < RING_TICKS; index++) {
    float angle = TAU * (float)index / RING_TICKS;
    bool passed =
        alarm || (view->run != AGENT_POMODORO_PENDING && angle <= sweep);
    draw_radial(angle, RING_TICK_INNER, RING_TICK_OUTER, 2,
                passed ? color : COLOR_MUTED, shift);
  }
  draw_radial(sweep, RING_HAND_INNER, RING_HAND_OUTER, 3, color, shift);
}

/// One line of the daily record: "3 FOCUS 1H15". Under an hour, only minutes.
static void format_tally(char *out, size_t size, unsigned completed,
                         uint32_t focus_s) {
  unsigned minutes = (unsigned)(focus_s / 60u);
  if (minutes < 60u) {
    snprintf(out, size, "%u FOCUS %uM", completed % 10000u, minutes % 60u);
  } else {
    snprintf(out, size, "%u FOCUS %uH%02u", completed % 10000u,
             (minutes / 60u) % 1000u, minutes % 60u);
  }
}

/// Rounds up to the second: shows 25:00 at the start and still 00:01 in the last millisecond.
static void format_countdown(char *out, size_t size, uint32_t remaining_ms) {
  uint32_t seconds = (remaining_ms + 999u) / 1000u;
  snprintf(out, size, "%02u:%02u", (unsigned)(seconds / 60u) % 100u,
           (unsigned)(seconds % 60u));
}

/// In the pomodoro scene the agents keep only a few lines at the bottom right: they still answer "what needs my attention most",
/// and voice lines still play; the screen just goes to the countdown.
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
  bool alarm = ring_alarm_active(&view);
  draw_pomodoro_ring(&view, alarm);

  // While paused the digits blink, the old stopwatch rule. While the alarm shakes, the digits shake with the ring.
  if (!paused || animation_frame % 2 == 0) {
    char countdown[8];
    format_countdown(countdown, sizeof(countdown), view.remaining_ms);
    draw_text(RING_CENTER_X - 58 + ring_alarm_shift(), RING_CENTER_Y - 14,
              countdown, 4, COLOR_TEXT, 5);
  }

  // Idle shows READY; right after focus ends and before the break starts, show BREAK with 05:00
  // to tell it apart from idle: this screen is waiting for the break to start, not for focus to start.
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

  // Daily record: one cell per completed session, plus a line with the count and total focus time. Resets on the
  // Mac's local date and survives restarts.
  draw_text(PANEL_X, 118, "TODAY", 1, COLOR_MUTED, 18);
  unsigned shown = view.completed > 8 ? 8 : view.completed;
  for (unsigned index = 0; index < shown; index++) {
    fill_rect(PANEL_X + (int)index * 12, 130, 8, 8, COLOR_FOCUS);
  }
  if (view.completed > 8) {
    char more[8];
    snprintf(more, sizeof(more), "+%u", (view.completed - 8) % 1000u);
    draw_text(PANEL_X + 96, 130, more, 1, COLOR_FOCUS, sizeof(more));
  }
  char tally_line[24];
  format_tally(tally_line, sizeof(tally_line), view.completed, view.focus_s);
  draw_text(PANEL_X, 144, tally_line, 1,
            view.completed > 0 ? COLOR_FOCUS : COLOR_MUTED, 18);

  draw_agent_summary(label, status_color);
}

/// Small pomodoro badge at the top right of the buddy scene: switching back to watch the agents shouldn't hide the countdown.
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

/// Leisure mode: the buddy leaves its duty spot and plays skits in the middle of the screen. The status bar and
/// footer stay as usual; a sleeping buddy simply means "nothing is waiting for you".
typedef enum {
  EYES_OPEN,
  EYES_CLOSED,
  EYES_WIDE,
  EYES_HALF,
} pet_eyes_t;

typedef enum {
  MOUTH_SMILE,
  MOUTH_FLAT,
  MOUTH_OPEN,
} pet_mouth_t;

typedef struct {
  int x;
  int y;
  int gaze_x;
  int gaze_y;
  int antenna;
  int left_arm;
  int right_arm;
  int left_foot;
  int right_foot;
  pet_eyes_t eyes;
  pet_mouth_t mouth;
} pet_pose_t;

static void draw_pet_eyes(int x, int y, pet_eyes_t eyes, int gaze_x,
                          int gaze_y, uint16_t color) {
  int face_y = 79 + y;
  int left = 143 + x + gaze_x;
  int right = 169 + x + gaze_x;
  if (eyes == EYES_CLOSED) {
    fill_rect(left, face_y + 14, 8, 3, color);
    fill_rect(right, face_y + 14, 8, 3, color);
  } else if (eyes == EYES_HALF) {
    fill_rect(left, face_y + 12 + gaze_y, 8, 5, color);
    fill_rect(right, face_y + 12 + gaze_y, 8, 5, color);
  } else if (eyes == EYES_WIDE) {
    fill_rect(left - 1, face_y + 6 + gaze_y, 10, 13, color);
    fill_rect(right - 1, face_y + 6 + gaze_y, 10, 13, color);
  } else {
    fill_rect(left, face_y + 8 + gaze_y, 8, 10, color);
    fill_rect(right, face_y + 8 + gaze_y, 8, 10, color);
  }
}

static void draw_pet_mouth(int x, int y, pet_mouth_t mouth, uint16_t color) {
  int face_y = 79 + y;
  if (mouth == MOUTH_SMILE) {
    draw_line(154 + x, face_y + 24, 160 + x, face_y + 27, color);
    draw_line(160 + x, face_y + 27, 166 + x, face_y + 24, color);
  } else if (mouth == MOUTH_FLAT) {
    draw_line(154 + x, face_y + 25, 166 + x, face_y + 25, color);
  } else {
    fill_rect(155 + x, face_y + 21, 10, 8, color);
  }
}

static void draw_pet_pose(const pet_pose_t *pose) {
  draw_pet_body(pose->x, pose->y, pose->antenna, COLOR_READY, pose->left_arm,
                pose->right_arm);
  draw_pet_eyes(pose->x, pose->y, pose->eyes, pose->gaze_x, pose->gaze_y,
                COLOR_READY);
  draw_pet_mouth(pose->x, pose->y, pose->mouth, COLOR_READY);
  draw_pet_legs(pose->x, pose->y, pose->left_foot, pose->right_foot);
}

/// Eyes open and smiling, blinking every three seconds.
static pet_pose_t resting_pose(uint32_t frame) {
  pet_pose_t pose = {0};
  pose.eyes = frame % 24 == 23 ? EYES_CLOSED : EYES_OPEN;
  pose.mouth = MOUTH_SMILE;
  return pose;
}

static pet_pose_t sleeping_pose(uint32_t frame) {
  pet_pose_t pose = {0};
  pose.eyes = EYES_CLOSED;
  pose.mouth = MOUTH_FLAT;
  pose.y = (int)((frame / 8) % 2);
  return pose;
}

static void draw_sleeping_z(int x, int y, uint32_t frame) {
  draw_text(172 + x, 44 + y - (int)(frame % 16), "Z", 2, COLOR_READY, 1);
}

/// Plain idle between skits: standing, breathing, blinking.
static void skit_rest(uint32_t frame) {
  pet_pose_t pose = resting_pose(frame);
  pose.y = (int)((frame / 8) % 2);
  draw_pet_pose(&pose);
}

static void skit_sleep(uint32_t frame) {
  pet_pose_t pose = sleeping_pose(frame);
  draw_pet_pose(&pose);
  draw_sleeping_z(0, pose.y, frame);
}

/// Patrol: walk to the right, stop and glance at you, walk to the left, then come back.
static void skit_patrol(uint32_t frame) {
  pet_pose_t pose = resting_pose(frame);
  bool walking = true;
  if (frame < 24) {
    pose.x = 3 * (int)frame;
    pose.gaze_x = 3;
  } else if (frame < 36) {
    pose.x = 72;
    walking = false;
  } else if (frame < 72) {
    pose.x = 72 - 3 * (int)(frame - 36);
    pose.gaze_x = -3;
  } else if (frame < 84) {
    pose.x = -36;
    walking = false;
  } else {
    pose.x = -36 + 3 * (int)(frame - 84);
    pose.gaze_x = 3;
  }
  if (walking) {
    bool left_step = frame % 4 < 2;
    pose.left_foot = left_step ? 4 : 0;
    pose.right_foot = left_step ? 0 : 4;
    pose.y = left_step ? 0 : 1;
  }
  draw_pet_pose(&pose);
}

static void draw_ball(int cx, int cy, uint32_t frame) {
  fill_rect(cx - 4, cy - 4, 8, 8, COLOR_INPUT);
  fill_rect(cx - 3, cy - 5, 6, 10, COLOR_INPUT);
  fill_rect(cx - 5, cy - 3, 10, 6, COLOR_INPUT);
  // A dark dot circling around makes the ball look like it's rolling.
  static const int8_t SPIN[4][2] = {{-2, -2}, {2, -2}, {2, 2}, {-2, 2}};
  fill_rect(cx + SPIN[frame % 4][0] - 1, cy + SPIN[frame % 4][1] - 1, 2, 2,
            COLOR_BACKGROUND);
}

/// Ball: the ball rolls from the left to the feet, gets kicked away, bounces and rolls back, then is kicked off screen.
/// The ball always flies on the left of the body and never passes through it.
static void skit_ball(uint32_t frame) {
  const int ground = 149;
  const int at_foot = 116;
  int ball_x;
  int ball_y = ground;
  bool kicking = false;
  if (frame < 24) {
    ball_x = 20 + (at_foot - 20) * (int)frame / 24;
  } else if (frame < 52) {
    float t = (float)(frame - 24) / 28.0f;
    ball_x = at_foot - (int)(86.0f * t);
    ball_y = ground - (int)(50.0f * sinf(3.14159f * t));
  } else if (frame < 60) {
    float t = (float)(frame - 52) / 8.0f;
    ball_x = 30 - (int)(10.0f * t);
    ball_y = ground - (int)(16.0f * sinf(3.14159f * t));
  } else if (frame < 80) {
    ball_x = 20 + (at_foot - 20) * (int)(frame - 60) / 20;
  } else {
    float t = (float)(frame - 80) / 16.0f;
    ball_x = at_foot - (int)(150.0f * t);
    ball_y = ground - (int)(40.0f * sinf(3.14159f * t));
  }
  if ((frame >= 21 && frame < 27) || (frame >= 77 && frame < 83)) {
    kicking = true;
  }
  pet_pose_t pose = resting_pose(frame);
  pose.gaze_x = -3;
  pose.gaze_y = ball_y < ground - 20 ? -2 : 2;
  if (kicking) {
    pose.left_foot = 8;
    pose.mouth = MOUTH_OPEN;
  }
  if (frame >= 88) {
    pose.eyes = EYES_WIDE;
  }
  draw_pet_pose(&pose);
  draw_ball(ball_x, ball_y, frame);
}

/// Reading: holds up a book and scans it line by line, turns a page every few seconds, and gets startled by the plot midway.
static void skit_read(uint32_t frame) {
  pet_pose_t pose = resting_pose(frame);
  pose.left_arm = 10;
  pose.right_arm = 10;
  pose.gaze_y = 3;
  pose.gaze_x = (int)((frame / 2) % 6) - 2;
  bool surprised = frame >= 72 && frame < 80;
  if (surprised) {
    pose.eyes = EYES_WIDE;
    pose.gaze_x = 0;
    pose.gaze_y = 0;
    pose.mouth = MOUTH_OPEN;
  }
  draw_pet_pose(&pose);

  int book_x = 138;
  int book_y = 114;
  fill_rect(book_x, book_y, 44, 26, COLOR_TEXT);
  fill_rect(book_x + 21, book_y, 2, 26, COLOR_MUTED);
  for (int line = 0; line < 3; line++) {
    fill_rect(book_x + 4, book_y + 5 + line * 6, 14, 2, COLOR_MUTED);
    fill_rect(book_x + 26, book_y + 5 + line * 6, 14, 2, COLOR_MUTED);
  }
  if (frame % 40 >= 36) {
    fill_rect(book_x + 14, book_y - 6, 10, 32, COLOR_TEXT);
  }
  if (surprised) {
    draw_text(190, 44, "!", 3, COLOR_INPUT, 1);
  }
}

/// Counting stars: looks up and counts to seven, slower and slower, and falls asleep counting.
static void skit_stars(uint32_t frame) {
  static const uint16_t STARS[][2] = {
      {20, 36},  {48, 52},  {75, 40},  {100, 60}, {130, 34}, {200, 44},
      {230, 62}, {262, 38}, {290, 54}, {306, 70}, {170, 66}, {60, 72},
  };
  for (unsigned index = 0; index < sizeof(STARS) / sizeof(STARS[0]); index++) {
    if ((frame / 3 + index) % 4 != 0) {
      fill_rect(STARS[index][0], STARS[index][1], 2, 2,
                index % 3 == 0 ? COLOR_TEXT : COLOR_MUTED);
    }
  }
  pet_pose_t pose = resting_pose(frame);
  pose.gaze_y = -3;
  pose.mouth = MOUTH_FLAT;
  if (frame >= 56 && frame < 88) {
    pose.eyes = EYES_HALF;
  } else if (frame >= 88) {
    pose = sleeping_pose(frame);
  }
  draw_pet_pose(&pose);
  if (frame < 56) {
    char count[4];
    snprintf(count, sizeof(count), "%u", (unsigned)(frame / 8 + 1) % 10u);
    draw_text(200, 52, count, 2, COLOR_MUTED, sizeof(count));
  } else if (frame < 88) {
    draw_text(200, 52, "...", 2, COLOR_MUTED, 3);
  } else {
    draw_sleeping_z(0, pose.y, frame);
  }
}

/// Hide and seek: slips past the right edge of the screen until only a waving hand shows, leans half out for a look, ducks back,
/// and finally walks back.
static void skit_hide(uint32_t frame) {
  pet_pose_t pose = resting_pose(frame);
  if (frame < 12) {
    pose.x = 16 * (int)frame;
  } else if (frame < 28) {
    pose.x = 192;
    pose.left_arm = frame % 4 < 2 ? 0 : 6;
  } else if (frame < 36) {
    pose.x = 192 - 9 * (int)(frame - 28);
    pose.eyes = EYES_WIDE;
  } else if (frame < 52) {
    pose.x = 120;
    pose.eyes = frame == 44 ? EYES_CLOSED : EYES_WIDE;
    pose.mouth = MOUTH_OPEN;
  } else if (frame < 64) {
    pose.x = 120 + 6 * (int)(frame - 52);
  } else {
    pose.x = 192 - 12 * (int)(frame - 64);
  }
  draw_pet_pose(&pose);
}

/// Startled awake: asleep, then an exclamation mark jumps up, it looks left and right, yawns, and goes back to sleep.
static void skit_startle(uint32_t frame) {
  bool sleeping = frame < 24 || frame >= 56;
  pet_pose_t pose = sleeping ? sleeping_pose(frame) : resting_pose(frame);
  if (!sleeping) {
    if (frame < 28) {
      pose.eyes = EYES_WIDE;
      pose.mouth = MOUTH_OPEN;
      pose.y = -6;
      pose.antenna = 4;
    } else if (frame < 44) {
      pose.eyes = EYES_WIDE;
      pose.gaze_x = frame < 36 ? -3 : 3;
    } else {
      pose.eyes = EYES_HALF;
      pose.mouth = MOUTH_OPEN;
      pose.left_arm = 6;
    }
  }
  draw_pet_pose(&pose);
  if (sleeping) {
    draw_sleeping_z(0, pose.y, frame);
  }
  if (frame >= 24 && frame < 32) {
    draw_text(190, 44, "!", 3, COLOR_INPUT, 1);
  }
}

/// Sleep talking: asleep, with a string of small bubbles overhead showing today's stats.
static void skit_dream(uint32_t frame) {
  pet_pose_t pose = sleeping_pose(frame);
  draw_pet_pose(&pose);
  static const uint8_t BUBBLES[3][3] = {{176, 58, 3}, {186, 50, 4}, {196, 42, 5}};
  for (unsigned index = 0; index < 3; index++) {
    if ((frame / 4) % 4 > index) {
      fill_rect(BUBBLES[index][0], BUBBLES[index][1], BUBBLES[index][2],
                BUBBLES[index][2], COLOR_MUTED);
    }
  }
  fill_rect(204, 34, 100, 26, COLOR_MUTED);
  fill_rect(206, 36, 96, 22, 0x10a4);
  const char *line = "ZZZ";
  if (current_stat_count > 0) {
    line = current_stats[(frame / 32) % current_stat_count];
  }
  draw_text(212, 43, line, 1, COLOR_TEXT, 15);
}

static void draw_leisure_scene(const agent_leisure_view_t *view) {
  uint32_t frame = view->skit_frame;
  switch (view->skit) {
    case AGENT_SKIT_PATROL:
      skit_patrol(frame);
      break;
    case AGENT_SKIT_BALL:
      skit_ball(frame);
      break;
    case AGENT_SKIT_READ:
      skit_read(frame);
      break;
    case AGENT_SKIT_STARS:
      skit_stars(frame);
      break;
    case AGENT_SKIT_HIDE:
      skit_hide(frame);
      break;
    case AGENT_SKIT_STARTLE:
      skit_startle(frame);
      break;
    case AGENT_SKIT_DREAM:
      skit_dream(frame);
      break;
    case AGENT_SKIT_SLEEP:
      skit_sleep(frame);
      break;
    default:
      skit_rest(frame);
      break;
  }
  draw_pomodoro_badge();
}

static void draw_pet_scene(const char *label, uint16_t status_color) {
  if (current_task_count > 0) {
    draw_task_cards();
  }
  draw_buddy(link_lost ? AGENT_DISPLAY_OFFLINE : current_state,
                  animation_frame, current_task_count > 0 ? 96 : 0,
                  status_color);
  draw_text_centered(162, label,
                     current_state == AGENT_DISPLAY_INPUT_REQUIRED ? 2 : 3,
                     status_color);
  if (current_task_count > 0) {
    draw_text_centered(195, "LATEST ON TOP", 1, COLOR_MUTED);
  } else if (current_state == AGENT_DISPLAY_IDLE && !link_lost) {
    // No rotation while the link is lost: a moving screen looks alive, the exact opposite of NO LINK.
    draw_idle_line();
  } else if (current_title[0] != '\0') {
    draw_text_centered(195, current_title, 2, COLOR_TEXT);
  }
  draw_pomodoro_badge();
}

static void ensure_backlight(bool wanted) {
  if (backlight_on == wanted) {
    return;
  }
  if (set_backlight(wanted) == ESP_OK) {
    backlight_on = wanted;
  }
}

static esp_err_t render_current_state(void) {
  const char *label = state_label(current_state);
  uint16_t status_color = state_color(current_state);
  render_dim = false;

  agent_leisure_view_t leisure;
  if (current_mode == AGENT_MODE_LEISURE) {
    agent_leisure_view(clock_ms(), &leisure);
    if (leisure.lights_out) {
      // After sleeping long at night, turn off the backlight; a screen nobody watches needn't be drawn.
      ensure_backlight(false);
      return ESP_OK;
    }
    render_dim = leisure.dim;
  }
  ensure_backlight(true);

  fill_rect(0, 0, DISPLAY_WIDTH, DISPLAY_HEIGHT, COLOR_BACKGROUND);
  fill_rect(0, 0, DISPLAY_WIDTH, 4, status_color);
  draw_text_centered(12, "Vibe Buddy", 2, COLOR_TEXT);
  if (muted) {
    draw_text(8, 15, "MUTE", 1, COLOR_INPUT, 4);
  }
  if (current_mode == AGENT_MODE_POMODORO) {
    draw_pomodoro_scene(label, status_color);
  } else if (current_mode == AGENT_MODE_LEISURE) {
    draw_leisure_scene(&leisure);
  } else {
    draw_pet_scene(label, status_color);
  }
  draw_build_footer();
  return present();
}

static TickType_t animation_period(agent_display_state_t state) {
  if (current_mode == AGENT_MODE_LEISURE) {
    return pdMS_TO_TICKS(AGENT_LEISURE_FRAME_MS);
  }
  if (current_mode == AGENT_MODE_POMODORO) {
    // The countdown changes every second; blinking while paused needs a frame every half second; the alarm shake is faster still.
    if (ring_alarm_shaking()) {
      return pdMS_TO_TICKS(RING_ALARM_SHAKE_FRAME_MS);
    }
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
    strncpy(current_tasks[index].project,
            tasks[index].project == NULL ? "" : tasks[index].project,
            sizeof(current_tasks[index].project) - 1);
    current_tasks[index].project[sizeof(current_tasks[index].project) - 1] = '\0';
    current_tasks[index].state = tasks[index].state;
    current_tasks[index].elapsed_base = tasks[index].elapsed_s;
    current_tasks[index].received_tick = xTaskGetTickCount();
  }
  next_animation_at = xTaskGetTickCount() + animation_period(current_state);
  return render_current_state();
}

void agent_display_dump(void (*write_line)(const char *line)) {
  char line[200];
  snprintf(line, sizeof(line), "SHOT BEGIN %dx%d BACKLIGHT %s", DISPLAY_WIDTH,
           DISPLAY_HEIGHT, backlight_on ? "ON" : "OFF");
  write_line(line);
  const size_t total = (size_t)DISPLAY_WIDTH * DISPLAY_HEIGHT;
  size_t index = 0;
  int used = snprintf(line, sizeof(line), "SHOT");
  int runs = 0;
  while (index < total) {
    uint16_t color = framebuffer[index];
    size_t run = 1;
    while (index + run < total && framebuffer[index + run] == color &&
           run < 60000) {
      run++;
    }
    used += snprintf(line + used, sizeof(line) - (size_t)used, " %04x:%u",
                     (unsigned)color, (unsigned)run);
    index += run;
    if (++runs == 16 || index == total) {
      write_line(line);
      used = snprintf(line, sizeof(line), "SHOT");
      runs = 0;
    }
  }
  write_line("SHOT END");
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

/// The heartbeat comes every 5 seconds; don't redraw if the stamp hasn't changed.
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

void agent_display_identify(void) {
  if (!display_ready) {
    return;
  }
  identify_until = xTaskGetTickCount() + pdMS_TO_TICKS(1200);
  identify_next_toggle = xTaskGetTickCount();
}

void agent_display_pomodoro_ended(void) {
  agent_pomodoro_view_t view;
  agent_pomodoro_view(clock_ms(), &view);
  ring_alarm = true;
  ring_alarm_phase = view.phase;
  ring_alarm_shake_frames = RING_ALARM_SHAKE_FRAMES;
  // The caller redraws the first frame right after; this only schedules the later frames on the shake rhythm.
  animation_frame = 0;
  next_animation_at =
      xTaskGetTickCount() + pdMS_TO_TICKS(RING_ALARM_SHAKE_FRAME_MS);
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

void agent_display_set_mode(agent_mode_t mode) {
  if (current_mode == mode) {
    return;
  }
  current_mode = mode;
  animation_frame = 0;
  // Switching away from the pomodoro screen means it was seen: the alarm can stop.
  if (mode != AGENT_MODE_POMODORO) {
    ring_alarm = false;
    ring_alarm_shake_frames = 0;
  }
  next_animation_at = xTaskGetTickCount() + animation_period(current_state);
  if (display_ready) {
    (void)render_current_state();
  }
}

agent_mode_t agent_display_mode(void) { return current_mode; }

void agent_display_set_muted(bool value) {
  if (muted == value) {
    return;
  }
  muted = value;
  if (display_ready) {
    (void)render_current_state();
  }
}

bool agent_display_agent_idle(void) {
  return current_state == AGENT_DISPLAY_IDLE && current_task_count == 0;
}

void agent_display_refresh(void) {
  if (display_ready) {
    (void)render_current_state();
  }
}

void agent_display_tick(void) {
  if (!display_ready) {
    return;
  }
  if (identify_until != 0) {
    TickType_t now = xTaskGetTickCount();
    if ((int32_t)(now - identify_until) >= 0) {
      identify_until = 0;
      ensure_backlight(true);
      next_animation_at = now;
    } else if ((int32_t)(now - identify_next_toggle) >= 0) {
      if (set_backlight(!backlight_on) == ESP_OK) {
        backlight_on = !backlight_on;
      }
      identify_next_toggle = now + pdMS_TO_TICKS(150);
    }
  }
  if ((int32_t)(xTaskGetTickCount() - next_animation_at) < 0) {
    return;
  }

  animation_frame++;
  if (ring_alarm_shake_frames > 0) {
    ring_alarm_shake_frames--;
  }
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
                      "failed to init I2C");

  i2c_device_config_t xl9555_config = {
      .dev_addr_length = I2C_ADDR_BIT_LEN_7,
      .device_address = XL9555_ADDRESS,
      .scl_speed_hz = 400000,
  };
  ESP_RETURN_ON_ERROR(
      i2c_master_bus_add_device(i2c_bus, &xl9555_config, &xl9555_handle), TAG,
      "failed to add XL9555");

  uint8_t direction;
  ESP_RETURN_ON_ERROR(xl9555_read(XL9555_CONFIG_PORT0, &direction), TAG,
                      "failed to probe XL9555");
  direction &= (uint8_t)~XL9555_LCD_BACKLIGHT_MASK;
  ESP_RETURN_ON_ERROR(xl9555_write(XL9555_CONFIG_PORT0, direction), TAG,
                      "failed to configure LCD backlight direction");
  ESP_RETURN_ON_ERROR(set_backlight(false), TAG, "failed to turn off LCD backlight");

  gpio_config_t read_pin_config = {
      .pin_bit_mask = 1ULL << LCD_NUM_RD,
      .mode = GPIO_MODE_INPUT_OUTPUT,
      .pull_up_en = GPIO_PULLUP_ENABLE,
  };
  ESP_RETURN_ON_ERROR(gpio_config(&read_pin_config), TAG, "failed to configure LCD RD");
  ESP_RETURN_ON_ERROR(gpio_set_level(LCD_NUM_RD, 1), TAG, "failed to drive LCD RD high");

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
                      "failed to init LCD i80 bus");

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
                      TAG, "failed to init LCD panel IO");

  esp_lcd_panel_dev_config_t panel_config = {
      .reset_gpio_num = GPIO_NUM_NC,
      .rgb_ele_order = LCD_RGB_ELEMENT_ORDER_RGB,
      .bits_per_pixel = 16,
  };
  ESP_RETURN_ON_ERROR(
      esp_lcd_new_panel_st7789(panel_io, &panel_config, &panel_handle), TAG,
      "failed to init ST7789");
  ESP_RETURN_ON_ERROR(esp_lcd_panel_reset(panel_handle), TAG,
                      "failed to reset ST7789");
  ESP_RETURN_ON_ERROR(esp_lcd_panel_init(panel_handle), TAG,
                      "failed to configure ST7789");
  ESP_RETURN_ON_ERROR(esp_lcd_panel_invert_color(panel_handle, true), TAG,
                      "failed to set LCD color inversion");
  ESP_RETURN_ON_ERROR(esp_lcd_panel_set_gap(panel_handle, 0, 0), TAG,
                      "failed to set LCD offset");

  uint8_t memory_access = 0x00;
  uint8_t pixel_format = 0x65;
  ESP_RETURN_ON_ERROR(
      esp_lcd_panel_io_tx_param(panel_io, 0x36, &memory_access, 1), TAG,
      "failed to set LCD memory access");
  ESP_RETURN_ON_ERROR(
      esp_lcd_panel_io_tx_param(panel_io, 0x3a, &pixel_format, 1), TAG,
      "failed to set LCD pixel format");
  ESP_RETURN_ON_ERROR(esp_lcd_panel_swap_xy(panel_handle, true), TAG,
                      "failed to set LCD orientation");
  ESP_RETURN_ON_ERROR(esp_lcd_panel_mirror(panel_handle, true, false), TAG,
                      "failed to set LCD mirroring");
  ESP_RETURN_ON_ERROR(esp_lcd_panel_disp_on_off(panel_handle, true), TAG,
                      "failed to turn on LCD panel");

  display_ready = true;
  ESP_RETURN_ON_ERROR(agent_display_show(AGENT_DISPLAY_IDLE, NULL), TAG,
                      "failed to draw initial screen");
  ESP_RETURN_ON_ERROR(set_backlight(true), TAG, "failed to turn on LCD backlight");
  backlight_on = true;
  return ESP_OK;
}
