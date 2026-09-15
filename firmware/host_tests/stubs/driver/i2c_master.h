#pragma once
#include <stddef.h>
#include <stdint.h>
#include "esp_err.h"
typedef struct stub_i2c_device *i2c_master_dev_handle_t;
typedef struct stub_i2c_bus *i2c_master_bus_handle_t;
typedef enum { I2C_NUM_0 = 0 } i2c_port_t;
typedef enum { I2C_ADDR_BIT_LEN_7 = 0 } i2c_addr_bit_len_t;
#define I2C_CLK_SRC_DEFAULT 0
typedef struct { i2c_port_t i2c_port; int sda_io_num; int scl_io_num; int clk_source; int glitch_ignore_cnt; struct { unsigned enable_internal_pullup : 1; } flags; } i2c_master_bus_config_t;
typedef struct { i2c_addr_bit_len_t dev_addr_length; uint16_t device_address; uint32_t scl_speed_hz; } i2c_device_config_t;
static inline esp_err_t i2c_master_transmit_receive(i2c_master_dev_handle_t handle, const uint8_t *write, size_t write_size, uint8_t *read, size_t read_size, int timeout) { (void)handle; (void)write; (void)write_size; (void)read_size; (void)timeout; *read = 0xff; return ESP_OK; }
static inline esp_err_t i2c_master_transmit(i2c_master_dev_handle_t handle, const uint8_t *write, size_t write_size, int timeout) { (void)handle; (void)write; (void)write_size; (void)timeout; return ESP_OK; }
static inline esp_err_t i2c_new_master_bus(const i2c_master_bus_config_t *config, i2c_master_bus_handle_t *bus) { (void)config; *bus = NULL; return ESP_OK; }
static inline esp_err_t i2c_master_bus_add_device(i2c_master_bus_handle_t bus, const i2c_device_config_t *config, i2c_master_dev_handle_t *device) { (void)bus; (void)config; *device = NULL; return ESP_OK; }
