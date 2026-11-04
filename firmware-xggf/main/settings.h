#ifndef XGGF_SETTINGS_H
#define XGGF_SETTINGS_H

#include <stdbool.h>

#include <stdint.h>
void settings_pomodoro_load(void *p);
void settings_pomodoro_save(void *p);
unsigned settings_volume(void);
bool settings_cycle_volume(void);
void settings_init(void);
bool settings_cycle_brightness(void);

#endif
