#ifndef XGGF_TEST_LEDC_H
#define XGGF_TEST_LEDC_H
#include <stdint.h>
#define ESP_OK 0
#define LEDC_LOW_SPEED_MODE 0
#define LEDC_TIMER_10_BIT 10
#define LEDC_TIMER_0 0
#define LEDC_AUTO_CLK 0
#define LEDC_CHANNEL_4 4
#define LEDC_INTR_DISABLE 0
typedef struct { int speed_mode,duty_resolution,timer_num,freq_hz,clk_cfg; } ledc_timer_config_t;
typedef struct { int gpio_num,speed_mode,channel,intr_type,timer_sel,duty,hpoint; } ledc_channel_config_t;
int ledc_timer_config(const ledc_timer_config_t *config);
int ledc_channel_config(const ledc_channel_config_t *config);
int ledc_set_duty(int mode,int channel,uint32_t duty);
int ledc_update_duty(int mode,int channel);
uint32_t ledc_set_freq(int mode,int timer,uint32_t frequency);
#endif
