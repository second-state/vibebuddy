#include "microphone.h"
#include <stdio.h>
#include "driver/i2s.h"

enum { PROBE_MS = 10000, REPORT_MS = 250, SAMPLE_CAPACITY = 128 };
static bool ready, probing, level_pending;
static uint32_t probe_started, probe_reported, latest_mean, latest_peak;
static int32_t samples[SAMPLE_CAPACITY];

bool microphone_init(void)
{
    i2s_config_t config = {
        .mode = I2S_MODE_MASTER | I2S_MODE_RX,
        .sample_rate = 8000,
        .bits_per_sample = I2S_BITS_PER_SAMPLE_32BIT,
        .channel_format = I2S_CHANNEL_FMT_ONLY_LEFT,
        .communication_format = I2S_COMM_FORMAT_STAND_I2S,
        .intr_alloc_flags = 0,
        .dma_buf_count = 8,
        .dma_buf_len = 64,
        .use_apll = false,
        .tx_desc_auto_clear = false,
        .fixed_mclk = 0,
    };
    i2s_pin_config_t pins = {
        .mck_io_num = I2S_PIN_NO_CHANGE,
        .bck_io_num = 10,
        .ws_io_num = 6,
        .data_out_num = I2S_PIN_NO_CHANGE,
        .data_in_num = 1,
    };
    if (i2s_driver_install(I2S_NUM_0, &config, 0, NULL) != ESP_OK) return false;
    if (i2s_set_pin(I2S_NUM_0, &pins) != ESP_OK ||
        i2s_set_clk(I2S_NUM_0, 8000, I2S_BITS_PER_SAMPLE_32BIT, I2S_CHANNEL_STEREO) != ESP_OK ||
        i2s_start(I2S_NUM_0) != ESP_OK) {
        i2s_driver_uninstall(I2S_NUM_0); return false;
    }
    i2s_zero_dma_buffer(I2S_NUM_0);
    ready = true;
    return true;
}

bool microphone_set_rate(uint32_t rate)
{
    if (!ready || (rate != 8000 && rate != 16000)) return false;
    if (i2s_set_clk(I2S_NUM_0, rate, I2S_BITS_PER_SAMPLE_32BIT, I2S_CHANNEL_STEREO) != ESP_OK) return false;
    probing = false; level_pending = false;
    i2s_zero_dma_buffer(I2S_NUM_0);
    return true;
}

size_t microphone_read(int32_t *output, size_t capacity)
{
    if (!ready || !output || !capacity) return 0;
    size_t bytes = 0;
    if (i2s_read(I2S_NUM_0, output, capacity * sizeof(output[0]), &bytes, 0) != ESP_OK) return 0;
    return bytes / sizeof(output[0]);
}

void microphone_probe(uint32_t now)
{
    if (!ready) return;
    probing = true; level_pending = false; probe_started = now; probe_reported = now;
}

void microphone_stop(void)
{
    probing = false;
    level_pending = false;
}

bool microphone_active(void)
{
    return probing;
}

void microphone_tick(uint32_t now)
{
    if (!ready) return;
    size_t bytes = 0;
    if (i2s_read(I2S_NUM_0, samples, sizeof(samples), &bytes, 0) != ESP_OK || bytes == 0) return;
    if (!probing) return;
    if ((uint32_t)(now - probe_started) >= PROBE_MS) {
        probing = false; level_pending = false; puts("MIC DONE"); return;
    }
    size_t count = bytes / sizeof(samples[0]);
    uint64_t total = 0;
    uint32_t peak = 0;
    for (size_t i = 0; i < count; ++i) {
        int32_t value = samples[i] >> 8;
        uint32_t amplitude = value < 0 ? (uint32_t)-value : (uint32_t)value;
        total += amplitude;
        if (amplitude > peak) peak = amplitude;
    }
    latest_mean = (uint32_t)(total / count); latest_peak = peak; level_pending = true;
    if ((uint32_t)(now - probe_reported) >= REPORT_MS) {
        probe_reported = now;
        printf("MIC mean=%u peak=%u samples=%u\n", (unsigned)latest_mean, (unsigned)peak, (unsigned)count);
    }
}

bool microphone_take_level(uint32_t *mean, uint32_t *peak)
{
    if (!level_pending || !mean || !peak) return false;
    *mean = latest_mean; *peak = latest_peak; level_pending = false;
    return true;
}
