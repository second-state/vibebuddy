#ifndef XGGF_BUZZER_H
#define XGGF_BUZZER_H
#include <stdbool.h>
#include <stdint.h>

bool buzzer_init(void);
void buzzer_identify(uint32_t now);
void buzzer_tick(uint32_t now);
#endif
