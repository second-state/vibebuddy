#include "agent_buttons.h"

#include <stdbool.h>
#include <stdint.h>

#include "driver/gpio.h"
#include "driver/i2c_master.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

#define XL9555_ADDRESS 0x20
#define XL9555_INPUT_PORT0 0x00
#define XL9555_CONFIG_PORT0 0x06
// ATK-DNESP32S3-BOX V1.1 实体按压确认：K2 为 P0.3、K1 为 P0.4，均低电平有效。
#define XL9555_K2_MASK 0x08
#define XL9555_K1_MASK 0x10
// K0 是 ESP32-S3 的 BOOT 键，直连 GPIO0，低电平有效；PCB 丝印写作 B0。
#define K0_GPIO GPIO_NUM_0
/// 一次翻转生效后，这么久之内不再接受第二次翻转。
#define DEBOUNCE_MS 40
#define LONG_PRESS_MS 1000

typedef struct {
  bool pressed;
  TickType_t changed_at;
  TickType_t pressed_at;
  /// 长按已经触发过，松开时就不再算一次短按。
  bool long_fired;
} button_t;

typedef enum {
  PRESS_NONE,
  PRESS_SHORT,
  PRESS_LONG,
} press_t;

static i2c_master_dev_handle_t xl9555_handle;
static agent_button_callback_t callback;
static bool buttons_ready;
static button_t k0;
static button_t k1;
static button_t k2;

static esp_err_t xl9555_read(uint8_t reg, uint8_t *value) {
  return i2c_master_transmit_receive(xl9555_handle, &reg, 1, value, 1,
                                     pdMS_TO_TICKS(100));
}

static esp_err_t xl9555_write(uint8_t reg, uint8_t value) {
  uint8_t command[] = {reg, value};
  return i2c_master_transmit(xl9555_handle, command, sizeof(command),
                             pdMS_TO_TICKS(100));
}

static void button_reset(button_t *button, bool pressed, TickType_t now) {
  button->pressed = pressed;
  button->changed_at = now;
  button->pressed_at = now;
  button->long_fired = false;
}

/// 翻转立即生效，只在翻转后 DEBOUNCE_MS 内忽略再次翻转。机械抖动只有几毫秒，
/// 而主循环每 20 ms 才采样一次；要求两次采样一致并不能多滤掉什么抖动，
/// 却会把一次短促的轻点整个丢掉——第一次实机验收时 K1 有一半按了没反应。
///
/// 短按在松开时才算数：只有等到松开，才知道它不是一次长按的开头。
static press_t button_update(button_t *button, bool pressed, TickType_t now) {
  if (pressed != button->pressed) {
    if ((int32_t)(now - button->changed_at) <
        (int32_t)pdMS_TO_TICKS(DEBOUNCE_MS)) {
      return PRESS_NONE;
    }
    button->pressed = pressed;
    button->changed_at = now;
    if (pressed) {
      button->pressed_at = now;
      button->long_fired = false;
      return PRESS_NONE;
    }
    return button->long_fired ? PRESS_NONE : PRESS_SHORT;
  }
  if (pressed && !button->long_fired &&
      (int32_t)(now - button->pressed_at) >=
          (int32_t)pdMS_TO_TICKS(LONG_PRESS_MS)) {
    button->long_fired = true;
    return PRESS_LONG;
  }
  return PRESS_NONE;
}

static bool k0_pressed(void) { return gpio_get_level(K0_GPIO) == 0; }

esp_err_t agent_buttons_init(agent_button_callback_t on_event) {
  gpio_config_t k0_config = {
      .pin_bit_mask = 1ULL << K0_GPIO,
      .mode = GPIO_MODE_INPUT,
      .pull_up_en = GPIO_PULLUP_ENABLE,
      .pull_down_en = GPIO_PULLDOWN_DISABLE,
      .intr_type = GPIO_INTR_DISABLE,
  };
  esp_err_t result = gpio_config(&k0_config);
  if (result != ESP_OK) {
    return result;
  }

  i2c_master_bus_handle_t i2c_bus;
  result = i2c_master_get_bus_handle(I2C_NUM_0, &i2c_bus);
  if (result != ESP_OK) {
    return result;
  }

  i2c_device_config_t device_config = {
      .dev_addr_length = I2C_ADDR_BIT_LEN_7,
      .device_address = XL9555_ADDRESS,
      .scl_speed_hz = 400000,
  };
  result = i2c_master_bus_add_device(i2c_bus, &device_config, &xl9555_handle);
  if (result != ESP_OK) {
    return result;
  }

  uint8_t direction;
  result = xl9555_read(XL9555_CONFIG_PORT0, &direction);
  if (result != ESP_OK) {
    return result;
  }
  direction |= XL9555_K2_MASK | XL9555_K1_MASK;
  result = xl9555_write(XL9555_CONFIG_PORT0, direction);
  if (result != ESP_OK) {
    return result;
  }

  uint8_t port0;
  result = xl9555_read(XL9555_INPUT_PORT0, &port0);
  if (result != ESP_OK) {
    return result;
  }
  TickType_t now = xTaskGetTickCount();
  button_reset(&k0, k0_pressed(), now);
  button_reset(&k1, (port0 & XL9555_K1_MASK) == 0, now);
  button_reset(&k2, (port0 & XL9555_K2_MASK) == 0, now);
  callback = on_event;
  buttons_ready = true;
  return ESP_OK;
}

static void emit(press_t press, agent_button_event_t short_event,
                 agent_button_event_t long_event, bool long_bound) {
  if (callback == NULL || press == PRESS_NONE) {
    return;
  }
  if (press == PRESS_SHORT) {
    callback(short_event);
  } else if (long_bound) {
    callback(long_event);
  }
}

void agent_buttons_tick(void) {
  if (!buttons_ready) {
    return;
  }

  TickType_t now = xTaskGetTickCount();
  emit(button_update(&k0, k0_pressed(), now), AGENT_BUTTON_K0_SHORT,
       AGENT_BUTTON_K0_LONG, true);

  uint8_t port0;
  if (xl9555_read(XL9555_INPUT_PORT0, &port0) != ESP_OK) {
    return;
  }
  emit(button_update(&k1, (port0 & XL9555_K1_MASK) == 0, now),
       AGENT_BUTTON_K1_SHORT, AGENT_BUTTON_K1_SHORT, false);
  emit(button_update(&k2, (port0 & XL9555_K2_MASK) == 0, now),
       AGENT_BUTTON_K2_SHORT, AGENT_BUTTON_K2_SHORT, false);
}
