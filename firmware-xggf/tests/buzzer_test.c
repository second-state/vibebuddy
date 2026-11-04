#include "buzzer.h"
#include "driver/ledc.h"
#include <assert.h>
static unsigned frequency,duty;
int ledc_timer_config(const ledc_timer_config_t *config) { frequency=config->freq_hz; return ESP_OK; }
int ledc_channel_config(const ledc_channel_config_t *config) { duty=config->duty; return ESP_OK; }
int ledc_set_duty(int mode,int channel,uint32_t value) { (void)mode;(void)channel;duty=value;return ESP_OK; }
int ledc_update_duty(int mode,int channel) { (void)mode;(void)channel;return ESP_OK; }
uint32_t ledc_set_freq(int mode,int timer,uint32_t value) { (void)mode;(void)timer;frequency=value;return value; }
int main(void) {
    assert(buzzer_init()); assert(duty==0);
    buzzer_volume(20); buzzer_chime(1,0);
    assert(frequency==1200 && duty==102);
    buzzer_tick(79); assert(frequency==1200 && duty>0);
    buzzer_tick(80); assert(frequency==1000 && duty>0);
    buzzer_tick(160); assert(duty==0);
    buzzer_chime(2,200); assert(frequency==800);
    buzzer_tick(280); assert(frequency==1000);
    buzzer_tick(360); assert(frequency==1200);
    buzzer_tick(440); assert(duty==0);
    buzzer_chime(2,500); buzzer_mute(true); assert(duty==0);
    buzzer_identify(600); buzzer_tick(1000); assert(duty==0);
    buzzer_mute(false); buzzer_volume(100); buzzer_identify(UINT32_MAX-20);
    assert(frequency==1000 && duty==512);
    buzzer_tick(59); assert(duty==0);
    buzzer_chime(2,1000); buzzer_identify(1010);
    assert(frequency==1000); buzzer_tick(1090); assert(duty==0);
}
