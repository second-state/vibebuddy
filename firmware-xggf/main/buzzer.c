#include "buzzer.h"
#include "driver/ledc.h"

enum { TONE_MS = 80, DUTY_SILENT = 0, DUTY_TONE = 512 };
static bool ready, sounding;
static uint32_t tone_started;

bool buzzer_init(void)
{
    ledc_timer_config_t timer = {
        .speed_mode = LEDC_LOW_SPEED_MODE,
        .duty_resolution = LEDC_TIMER_10_BIT,
        .timer_num = LEDC_TIMER_0,
        .freq_hz = 1000,
        .clk_cfg = LEDC_AUTO_CLK,
    };
    if (ledc_timer_config(&timer) != ESP_OK) return false;
    ledc_channel_config_t channel = {
        .gpio_num = 3,
        .speed_mode = LEDC_LOW_SPEED_MODE,
        .channel = LEDC_CHANNEL_4,
        .intr_type = LEDC_INTR_DISABLE,
        .timer_sel = LEDC_TIMER_0,
        .duty = DUTY_SILENT,
        .hpoint = 0,
    };
    ready = ledc_channel_config(&channel) == ESP_OK;
    return ready;
}

void buzzer_identify(uint32_t now)
{
    if (!ready) return;
    if (ledc_set_duty(LEDC_LOW_SPEED_MODE, LEDC_CHANNEL_4, DUTY_TONE) != ESP_OK) return;
    if (ledc_update_duty(LEDC_LOW_SPEED_MODE, LEDC_CHANNEL_4) != ESP_OK) return;
    tone_started = now; sounding = true;
}

void buzzer_tick(uint32_t now)
{
    if (!sounding || (uint32_t)(now - tone_started) < TONE_MS) return;
    ledc_set_duty(LEDC_LOW_SPEED_MODE, LEDC_CHANNEL_4, DUTY_SILENT);
    ledc_update_duty(LEDC_LOW_SPEED_MODE, LEDC_CHANNEL_4);
    sounding = false;
}
