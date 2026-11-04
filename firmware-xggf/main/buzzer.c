#include "buzzer.h"
#include "driver/ledc.h"

enum { TONE_MS = 80, DUTY_SILENT = 0, DUTY_TONE = 512 };
static bool ready, sounding, muted;
static unsigned volume=50, note, notes;
static unsigned frequencies[3];
void buzzer_volume(unsigned level)
{
    volume = level <= 100 ? level : 100;
}

void buzzer_mute(bool value)
{
    muted = value;
    if (value && ready) {
        ledc_set_duty(LEDC_LOW_SPEED_MODE, LEDC_CHANNEL_4, DUTY_SILENT);
        ledc_update_duty(LEDC_LOW_SPEED_MODE, LEDC_CHANNEL_4);
        sounding = false;
        notes = 0;
    }
}

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

static void start_tone(uint32_t now)
{
    if (!ready || muted) return;
    ledc_set_freq(LEDC_LOW_SPEED_MODE, LEDC_TIMER_0, frequencies[note]);
    if (ledc_set_duty(LEDC_LOW_SPEED_MODE, LEDC_CHANNEL_4, DUTY_TONE * volume / 100) != ESP_OK) return;
    if (ledc_update_duty(LEDC_LOW_SPEED_MODE, LEDC_CHANNEL_4) != ESP_OK) return;
    tone_started = now; sounding = true;
}

void buzzer_identify(uint32_t now)
{
    note = 0;
    notes = 1;
    frequencies[0] = 1000;
    start_tone(now);
}

void buzzer_tick(uint32_t now)
{
    if (!sounding || (uint32_t)(now - tone_started) < TONE_MS) return;
    if (notes && ++note < notes) {
        ledc_set_freq(LEDC_LOW_SPEED_MODE,LEDC_TIMER_0,frequencies[note]);
        tone_started=now; return;
    }
    notes=0;
    ledc_set_duty(LEDC_LOW_SPEED_MODE, LEDC_CHANNEL_4, DUTY_SILENT);
    ledc_update_duty(LEDC_LOW_SPEED_MODE, LEDC_CHANNEL_4);
    sounding = false;
}

void buzzer_chime(unsigned kind, uint32_t now)
{
    if (!ready || muted) return;
    note=0; notes=kind==1 ? 2 : 3;
    frequencies[0]=kind==1 ? 1200 : 800;
    frequencies[1]=1000; frequencies[2]=1200;
    ledc_set_freq(LEDC_LOW_SPEED_MODE,LEDC_TIMER_0,frequencies[0]);
    start_tone(now);
}
