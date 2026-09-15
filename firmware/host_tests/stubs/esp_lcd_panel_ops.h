#pragma once
#include <stdbool.h>
#include "esp_err.h"
typedef struct stub_panel *esp_lcd_panel_handle_t;
static inline esp_err_t esp_lcd_panel_draw_bitmap(esp_lcd_panel_handle_t panel, int x0, int y0, int x1, int y1, const void *data) { (void)panel; (void)x0; (void)y0; (void)x1; (void)y1; (void)data; return ESP_OK; }
static inline esp_err_t esp_lcd_panel_reset(esp_lcd_panel_handle_t panel) { (void)panel; return ESP_OK; }
static inline esp_err_t esp_lcd_panel_init(esp_lcd_panel_handle_t panel) { (void)panel; return ESP_OK; }
static inline esp_err_t esp_lcd_panel_invert_color(esp_lcd_panel_handle_t panel, bool invert) { (void)panel; (void)invert; return ESP_OK; }
static inline esp_err_t esp_lcd_panel_set_gap(esp_lcd_panel_handle_t panel, int x, int y) { (void)panel; (void)x; (void)y; return ESP_OK; }
static inline esp_err_t esp_lcd_panel_swap_xy(esp_lcd_panel_handle_t panel, bool swap) { (void)panel; (void)swap; return ESP_OK; }
static inline esp_err_t esp_lcd_panel_mirror(esp_lcd_panel_handle_t panel, bool x, bool y) { (void)panel; (void)x; (void)y; return ESP_OK; }
static inline esp_err_t esp_lcd_panel_disp_on_off(esp_lcd_panel_handle_t panel, bool on) { (void)panel; (void)on; return ESP_OK; }
