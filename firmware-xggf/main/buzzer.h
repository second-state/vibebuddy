#ifndef XGGF_BUZZER_H
#define XGGF_BUZZER_H
#include <stdbool.h>
#include <stdint.h>

void buzzer_volume(unsigned level);
void buzzer_mute(bool muted);
void buzzer_chime(unsigned kind, uint32_t now);
bool buzzer_init(void);
void buzzer_identify(uint32_t now);
void buzzer_tick(uint32_t now);
#endif
