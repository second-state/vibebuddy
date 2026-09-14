#include "agent_display.h"

#include <ctype.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

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

#define TITLE_BYTES 64

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
static char current_title[TITLE_BYTES];
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

static void draw_pet_face(agent_display_state_t state, uint32_t frame,
                          int y_offset, uint16_t color) {
  int face_y = 79 + y_offset;
  if (state == AGENT_DISPLAY_WORKING) {
    draw_text(139, face_y + 5, ">", 2, color, 1);
    int dot_count = (int)(frame % 3) + 1;
    for (int index = 0; index < dot_count; index++) {
      fill_rect(163 + index * 8, face_y + 17, 5, 3, color);
    }
  } else if (state == AGENT_DISPLAY_INPUT_REQUIRED) {
    draw_text(141, face_y + 5, "!?", 2, color, 2);
  } else if (state == AGENT_DISPLAY_DONE) {
    draw_line(140, face_y + 12, 147, face_y + 6, color);
    draw_line(147, face_y + 6, 154, face_y + 12, color);
    draw_line(166, face_y + 12, 173, face_y + 6, color);
    draw_line(173, face_y + 6, 180, face_y + 12, color);
    draw_line(151, face_y + 19, 160, face_y + 23, color);
    draw_line(160, face_y + 23, 169, face_y + 19, color);
  } else if (state == AGENT_DISPLAY_FAILED) {
    draw_line(140, face_y + 7, 153, face_y + 18, color);
    draw_line(153, face_y + 7, 140, face_y + 18, color);
    draw_line(167, face_y + 7, 180, face_y + 18, color);
    draw_line(180, face_y + 7, 167, face_y + 18, color);
    draw_line(153, face_y + 25, 167, face_y + 25, color);
  } else {
    bool blinking = frame % 8 == 7;
    fill_rect(143, face_y + (blinking ? 14 : 8), 8, blinking ? 3 : 10, color);
    fill_rect(169, face_y + (blinking ? 14 : 8), 8, blinking ? 3 : 10, color);
    draw_line(154, face_y + 24, 160, face_y + 27, color);
    draw_line(160, face_y + 27, 166, face_y + 24, color);
  }
}

static void draw_beaconling(agent_display_state_t state, uint32_t frame,
                            uint16_t color) {
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

  fill_rect(157, 48 + y_offset, 6, 13, COLOR_PET_HIGHLIGHT);
  fill_rect(153, 44 + y_offset, 14, 10, color);

  fill_rect(113, 65 + y_offset, 94, 52, COLOR_PET);
  fill_rect(121, 59 + y_offset, 78, 64, COLOR_PET);
  fill_rect(105, 78 + y_offset, 12, 28, COLOR_PET_HIGHLIGHT);
  fill_rect(203, 78 + y_offset, 12, 28, COLOR_PET_HIGHLIGHT);
  fill_rect(128, 75 + y_offset, 64, 40, COLOR_BACKGROUND);
  fill_rect(132, 79 + y_offset, 56, 32, 0x10a4);
  draw_pet_face(state, frame, y_offset, color);

  fill_rect(139, 121 + y_offset, 42, 25, COLOR_PET);
  fill_rect(126, 125 + y_offset, 13, 17, COLOR_PET_HIGHLIGHT);
  fill_rect(181, 125 + y_offset, 13, 17, COLOR_PET_HIGHLIGHT);
  fill_rect(143, 145 + y_offset, 13, 8, COLOR_PET_HIGHLIGHT);
  fill_rect(164, 145 + y_offset, 13, 8, COLOR_PET_HIGHLIGHT);
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

static esp_err_t render_current_state(void) {
  const char *label = "READY";
  uint16_t status_color = COLOR_READY;
  if (current_state == AGENT_DISPLAY_WORKING) {
    label = "WORKING";
    status_color = COLOR_WORKING;
  } else if (current_state == AGENT_DISPLAY_INPUT_REQUIRED) {
    label = "INPUT REQUIRED";
    status_color = COLOR_INPUT;
  } else if (current_state == AGENT_DISPLAY_DONE) {
    label = "DONE";
    status_color = COLOR_DONE;
  } else if (current_state == AGENT_DISPLAY_FAILED) {
    label = "FAILED";
    status_color = COLOR_FAILED;
  }

  fill_rect(0, 0, DISPLAY_WIDTH, DISPLAY_HEIGHT, COLOR_BACKGROUND);
  fill_rect(0, 0, DISPLAY_WIDTH, 4, status_color);
  draw_text_centered(12, "AgentBeacon", 2, COLOR_TEXT);
  draw_beaconling(current_state, animation_frame, status_color);
  draw_text_centered(162, label,
                     current_state == AGENT_DISPLAY_INPUT_REQUIRED ? 2 : 3,
                     status_color);
  if (current_title[0] != '\0') {
    draw_text_centered(195, current_title, 2, COLOR_TEXT);
  } else if (current_state == AGENT_DISPLAY_IDLE) {
    draw_text_centered(195, "YOUR AGENT PET", 2, COLOR_MUTED);
  }
  draw_text_centered(228, "BEACONLING  USB ONLINE", 1, COLOR_MUTED);
  return present();
}

static TickType_t animation_period(agent_display_state_t state) {
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
  next_animation_at = xTaskGetTickCount() + animation_period(current_state);
  return render_current_state();
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
