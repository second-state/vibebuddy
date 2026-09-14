#include "agent_buttons.h"

#include <stdbool.h>
#include <stdint.h>

#include "driver/i2c_master.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

#define XL9555_ADDRESS 0x20
#define XL9555_INPUT_PORT0 0x00
#define XL9555_CONFIG_PORT0 0x06
// ATK-DNESP32S3-BOX V1.1 实体按压确认：K2 为 P0.3，低电平有效。
#define XL9555_K2_MASK 0x08
#define DEBOUNCE_MS 60

static i2c_master_dev_handle_t xl9555_handle;
static agent_button_callback_t k2_callback;
static bool buttons_ready;
static bool raw_pressed;
static bool stable_pressed;
static TickType_t raw_changed_at;

static esp_err_t xl9555_read(uint8_t reg, uint8_t *value) {
  return i2c_master_transmit_receive(xl9555_handle, &reg, 1, value, 1,
                                     pdMS_TO_TICKS(100));
}

static esp_err_t xl9555_write(uint8_t reg, uint8_t value) {
  uint8_t command[] = {reg, value};
  return i2c_master_transmit(xl9555_handle, command, sizeof(command),
                             pdMS_TO_TICKS(100));
}

esp_err_t agent_buttons_init(agent_button_callback_t on_k2_pressed) {
  i2c_master_bus_handle_t i2c_bus;
  esp_err_t result = i2c_master_get_bus_handle(I2C_NUM_0, &i2c_bus);
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
  direction |= XL9555_K2_MASK;
  result = xl9555_write(XL9555_CONFIG_PORT0, direction);
  if (result != ESP_OK) {
    return result;
  }

  uint8_t input;
  result = xl9555_read(XL9555_INPUT_PORT0, &input);
  if (result != ESP_OK) {
    return result;
  }
  raw_pressed = (input & XL9555_K2_MASK) == 0;
  stable_pressed = raw_pressed;
  raw_changed_at = xTaskGetTickCount();
  k2_callback = on_k2_pressed;
  buttons_ready = true;
  return ESP_OK;
}

void agent_buttons_tick(void) {
  if (!buttons_ready) {
    return;
  }

  uint8_t input;
  if (xl9555_read(XL9555_INPUT_PORT0, &input) != ESP_OK) {
    return;
  }
  TickType_t now = xTaskGetTickCount();
  bool pressed = (input & XL9555_K2_MASK) == 0;
  if (pressed != raw_pressed) {
    raw_pressed = pressed;
    raw_changed_at = now;
    return;
  }
  if (pressed == stable_pressed ||
      (int32_t)(now - raw_changed_at) <
          (int32_t)pdMS_TO_TICKS(DEBOUNCE_MS)) {
    return;
  }

  stable_pressed = pressed;
  if (stable_pressed && k2_callback != NULL) {
    k2_callback();
  }
}
