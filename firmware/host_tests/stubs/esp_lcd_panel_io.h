#pragma once
#include <stddef.h>
#include <stdint.h>
#include "esp_err.h"
typedef struct stub_panel_io *esp_lcd_panel_io_handle_t;
typedef struct stub_i80_bus *esp_lcd_i80_bus_handle_t;
typedef struct { int unused; } esp_lcd_panel_io_event_data_t;
typedef bool (*esp_lcd_panel_io_color_trans_done_cb_t)(esp_lcd_panel_io_handle_t, esp_lcd_panel_io_event_data_t *, void *);
#define LCD_CLK_SRC_DEFAULT 0
typedef struct { int dc_gpio_num; int wr_gpio_num; int clk_src; int data_gpio_nums[8]; size_t bus_width; size_t max_transfer_bytes; size_t psram_trans_align; size_t sram_trans_align; } esp_lcd_i80_bus_config_t;
typedef struct { int cs_gpio_num; uint32_t pclk_hz; size_t trans_queue_depth; esp_lcd_panel_io_color_trans_done_cb_t on_color_trans_done; void *user_ctx; int lcd_cmd_bits; int lcd_param_bits; struct { unsigned dc_idle_level : 1; unsigned dc_cmd_level : 1; unsigned dc_dummy_level : 1; unsigned dc_data_level : 1; } dc_levels; struct { unsigned swap_color_bytes : 1; } flags; } esp_lcd_panel_io_i80_config_t;
static inline esp_err_t esp_lcd_new_i80_bus(const esp_lcd_i80_bus_config_t *config, esp_lcd_i80_bus_handle_t *bus) { (void)config; *bus = NULL; return ESP_OK; }
static inline esp_err_t esp_lcd_new_panel_io_i80(esp_lcd_i80_bus_handle_t bus, const esp_lcd_panel_io_i80_config_t *config, esp_lcd_panel_io_handle_t *io) { (void)bus; (void)config; *io = NULL; return ESP_OK; }
static inline esp_err_t esp_lcd_panel_io_tx_param(esp_lcd_panel_io_handle_t io, int command, const void *param, size_t size) { (void)io; (void)command; (void)param; (void)size; return ESP_OK; }
