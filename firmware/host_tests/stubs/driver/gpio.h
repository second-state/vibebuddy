#pragma once
#include <stdint.h>
#include "esp_err.h"
typedef enum { GPIO_NUM_NC = -1, GPIO_NUM_0 = 0, GPIO_NUM_1, GPIO_NUM_2, GPIO_NUM_9 = 9, GPIO_NUM_10, GPIO_NUM_11, GPIO_NUM_12, GPIO_NUM_38 = 38, GPIO_NUM_39, GPIO_NUM_40, GPIO_NUM_41, GPIO_NUM_42, GPIO_NUM_45 = 45, GPIO_NUM_46, GPIO_NUM_48 = 48 } gpio_num_t;
typedef enum { GPIO_MODE_INPUT_OUTPUT = 3 } gpio_mode_t;
typedef enum { GPIO_PULLUP_ENABLE = 1 } gpio_pullup_t;
typedef struct { uint64_t pin_bit_mask; gpio_mode_t mode; gpio_pullup_t pull_up_en; } gpio_config_t;
static inline esp_err_t gpio_config(const gpio_config_t *config) { (void)config; return ESP_OK; }
static inline esp_err_t gpio_set_level(gpio_num_t pin, uint32_t level) { (void)pin; (void)level; return ESP_OK; }
