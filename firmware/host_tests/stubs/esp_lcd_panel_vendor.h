#pragma once
#include "esp_lcd_panel_io.h"
#include "esp_lcd_panel_ops.h"
typedef enum { LCD_RGB_ELEMENT_ORDER_RGB = 0 } lcd_rgb_element_order_t;
typedef struct { int reset_gpio_num; lcd_rgb_element_order_t rgb_ele_order; unsigned bits_per_pixel; } esp_lcd_panel_dev_config_t;
static inline esp_err_t esp_lcd_new_panel_st7789(esp_lcd_panel_io_handle_t io, const esp_lcd_panel_dev_config_t *config, esp_lcd_panel_handle_t *panel) { (void)io; (void)config; *panel = NULL; return ESP_OK; }
