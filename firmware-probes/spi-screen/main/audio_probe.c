#include <stdint.h>
#include "driver/i2s_std.h"
#include "esp_check.h"
#include "esp_log.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "audio_probe.h"

extern const uint8_t clip_start[] asm("_binary_probe_done_start");
extern const uint8_t clip_end[] asm("_binary_probe_done_end");
static const char *TAG = "audio_probe";
static i2s_chan_handle_t tx;

static void write_samples(const int16_t *samples, size_t bytes) {
    size_t sent = 0;
    ESP_ERROR_CHECK(i2s_channel_write(tx, samples, bytes, &sent, 1000));
    ESP_ERROR_CHECK(sent == bytes ? ESP_OK : ESP_ERR_TIMEOUT);
}

static void play_task(void *argument) {
    (void)argument;
    int16_t samples[512];
    const size_t length = (size_t)(clip_end - clip_start);
    ESP_ERROR_CHECK(length % 4 == 0 ? ESP_OK : ESP_ERR_INVALID_SIZE);
    vTaskDelay(pdMS_TO_TICKS(1500));
    for (unsigned repeat = 1; repeat <= 3; ++repeat) {
        ESP_LOGI(TAG, "第 %u/3 次播放任务完成，数字幅度为原素材的 1/16", repeat);
        for (size_t offset = 0; offset < length;) {
            size_t count = (length - offset) / 2;
            if (count > 512) count = 512;
            for (size_t i = 0; i < count; ++i) {
                uint16_t raw = (uint16_t)clip_start[offset + 2 * i] |
                               ((uint16_t)clip_start[offset + 2 * i + 1] << 8);
                // 显式解码双声道 24 kHz、16-bit 小端 PCM，保留左右声道。
                int32_t signed_sample = raw < 0x8000 ? raw : (int32_t)raw - 65536;
                samples[i] = (int16_t)(signed_sample / 16);
            }
            write_samples(samples, count * sizeof(*samples));
            offset += count * sizeof(*samples);
        }
        // 送入静音并等待 DMA 消费完成；避免停止时截断语音。
        for (size_t i = 0; i < 512; ++i) samples[i] = 0;
        for (unsigned i = 0; i < 12; ++i) write_samples(samples, sizeof(samples));
        ESP_LOGI(TAG, "语音数据已提交，需人工确认实际声音及屏幕是否持续刷新");
        vTaskDelay(pdMS_TO_TICKS(5000));
    }
    ESP_ERROR_CHECK(i2s_channel_disable(tx));
    ESP_LOGI(TAG, "三次播放结束；按 RST 可重新测试");
    vTaskDelete(NULL);
}

void audio_probe_start(void) {
    i2s_chan_config_t channel = I2S_CHANNEL_DEFAULT_CONFIG(I2S_NUM_0, I2S_ROLE_MASTER);
    channel.auto_clear = true;
    ESP_ERROR_CHECK(i2s_new_channel(&channel, &tx, NULL));
    i2s_std_config_t config = {
        .clk_cfg = I2S_STD_CLK_DEFAULT_CONFIG(24000),
        .slot_cfg = I2S_STD_PHILIPS_SLOT_DEFAULT_CONFIG(I2S_DATA_BIT_WIDTH_16BIT,
                                                      I2S_SLOT_MODE_STEREO),
        .gpio_cfg = {
            .mclk = I2S_GPIO_UNUSED, .bclk = GPIO_NUM_5, .ws = GPIO_NUM_6,
            .dout = GPIO_NUM_7, .din = I2S_GPIO_UNUSED,
        },
    };
    ESP_ERROR_CHECK(i2s_channel_init_std_mode(tx, &config));
    ESP_ERROR_CHECK(i2s_channel_enable(tx));
    ESP_LOGI(TAG, "MAX98357A: BCLK=5 LRC=6 DIN=7；VIN/SD 接 3V3，GAIN 悬空");
    ESP_ERROR_CHECK(xTaskCreate(play_task, "audio_probe", 4096, NULL, 4, NULL) == pdPASS
                    ? ESP_OK : ESP_ERR_NO_MEM);
}
