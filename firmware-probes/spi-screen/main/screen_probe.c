#include <stdint.h>
#include <stdlib.h>
#include "driver/spi_master.h"
#include "esp_check.h"
#include "esp_heap_caps.h"
#include "esp_lcd_panel_io.h"
#include "esp_lcd_panel_ops.h"
#include "esp_lcd_panel_vendor.h"
#include "esp_log.h"
#include "freertos/FreeRTOS.h"
#include "freertos/semphr.h"
#include "freertos/task.h"
#include "audio_probe.h"

// 仅用于 GOOUUU S3-N16R8 + ST7789 SPI 240×240 面包板探针。
// VCC/BLK 接 3V3，GND 接 GND；不初始化按键及 BOX 外设。
enum { WIDTH = 240, HEIGHT = 240, ROWS = 16 };
static const char *TAG = "screen_probe";
static SemaphoreHandle_t transfer_done;
static esp_lcd_panel_handle_t panel;
static uint16_t *pixels;

static bool color_done(esp_lcd_panel_io_handle_t io,
                       esp_lcd_panel_io_event_data_t *event, void *context) {
    (void)io;
    (void)event;
    BaseType_t wake = pdFALSE;
    xSemaphoreGiveFromISR((SemaphoreHandle_t)context, &wake);
    return wake == pdTRUE;
}

static void draw_pattern(unsigned phase, unsigned frame) {
    const uint16_t colors[] = {0xf800, 0x07e0, 0x001f};
    for (int top = 0; top < HEIGHT; top += ROWS) {
        for (int row = 0; row < ROWS; ++row) {
            const int y = top + row;
            for (int x = 0; x < WIDTH; ++x) {
                uint16_t color = colors[phase < 3 ? phase : x / 80];
                if (phase == 3) {
                    // 白色完整边框、左上白块、右下黄块，用于检查裁切和方向。
                    if (x < 2 || x >= WIDTH - 2 || y < 2 || y >= HEIGHT - 2 ||
                        (x >= 8 && x < 24 && y >= 8 && y < 24)) color = 0xffff;
                    if (x >= 216 && x < 232 && y >= 216 && y < 232) color = 0xffe0;
                    int marker = 8 + (frame * 4) % 208;
                    if (x >= marker && x < marker + 12 && y >= 114 && y < 126)
                        color = 0xffff;
                }
                // RGB565 高字节先发；ESP32 内存为小端。
                pixels[row * WIDTH + x] = (uint16_t)((color << 8) | (color >> 8));
            }
        }
        ESP_ERROR_CHECK(esp_lcd_panel_draw_bitmap(panel, 0, top, WIDTH, top + ROWS, pixels));
        if (xSemaphoreTake(transfer_done, pdMS_TO_TICKS(2000)) != pdTRUE) {
            ESP_LOGE(TAG, "SPI 传输超时");
            abort();
        }
    }
}

void app_main(void) {
    ESP_LOGI(TAG, "GOOUUU SPI SCREEN PROBE: SCLK=12 MOSI=11 RST=10 DC=9 CS=8");
    transfer_done = xSemaphoreCreateBinary();
    pixels = heap_caps_malloc(WIDTH * ROWS * sizeof(*pixels), MALLOC_CAP_DMA);
    if (!transfer_done || !pixels) abort();
    spi_bus_config_t bus = {
        .sclk_io_num = 12, .mosi_io_num = 11, .miso_io_num = -1,
        .quadwp_io_num = -1, .quadhd_io_num = -1,
        .max_transfer_sz = WIDTH * ROWS * sizeof(*pixels),
    };
    ESP_ERROR_CHECK(spi_bus_initialize(SPI2_HOST, &bus, SPI_DMA_CH_AUTO));
    esp_lcd_panel_io_spi_config_t io_config = {
        .cs_gpio_num = 8, .dc_gpio_num = 9, .spi_mode = 0,
        .pclk_hz = 4 * 1000 * 1000, .trans_queue_depth = 1,
        .lcd_cmd_bits = 8, .lcd_param_bits = 8,
        .on_color_trans_done = color_done, .user_ctx = transfer_done,
    };
    esp_lcd_panel_io_handle_t io;
    ESP_ERROR_CHECK(esp_lcd_new_panel_io_spi((esp_lcd_spi_bus_handle_t)SPI2_HOST,
                                           &io_config, &io));
    esp_lcd_panel_dev_config_t config = {
        .reset_gpio_num = 10, .rgb_ele_order = LCD_RGB_ELEMENT_ORDER_RGB,
        .bits_per_pixel = 16,
    };
    ESP_ERROR_CHECK(esp_lcd_new_panel_st7789(io, &config, &panel));
    ESP_ERROR_CHECK(esp_lcd_panel_reset(panel));
    ESP_ERROR_CHECK(esp_lcd_panel_init(panel));
    ESP_ERROR_CHECK(esp_lcd_panel_invert_color(panel, true));
    ESP_ERROR_CHECK(esp_lcd_panel_set_gap(panel, 0, 0));
    ESP_ERROR_CHECK(esp_lcd_panel_disp_on_off(panel, true));
    for (unsigned phase = 0; phase < 3; ++phase) {
        draw_pattern(phase, 0);
        vTaskDelay(pdMS_TO_TICKS(1000));
    }
    ESP_LOGI(TAG, "图案已提交：左红、中绿、右蓝；白边框；左上白块；右下黄块");
    audio_probe_start();
    for (unsigned frame = 0;; ++frame) {
        draw_pattern(3, frame);
        if (frame % 100 == 0) ESP_LOGI(TAG, "刷新计数 %u（仍需肉眼确认画面）", frame);
        vTaskDelay(pdMS_TO_TICKS(100));
    }
}
