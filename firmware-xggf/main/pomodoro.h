#ifndef XGGF_POMODORO_H
#define XGGF_POMODORO_H
#include <stdint.h>
#include <stdbool.h>
typedef struct { uint32_t phase, run, remaining_ms, total_ms, completed, focus_s; } pomodoro_view;
void *vb_pomodoro_new(void);
void vb_pomodoro_free(void *p);
void vb_pomodoro_toggle(void *p, uint32_t now);
void vb_pomodoro_stop(void *p);
uint32_t vb_pomodoro_tick(void *p, uint32_t now);
void vb_pomodoro_view(const void *p, uint32_t now, pomodoro_view *out);
bool vb_pomodoro_set_day(void *p, uint32_t day);
void vb_pomodoro_tally(const void *p, uint32_t *day, uint32_t *count, uint32_t *secs);
void vb_pomodoro_restore(void *p, uint32_t day, uint32_t count, uint32_t secs);
#endif
