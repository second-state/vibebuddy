#ifndef XGGF_MICROPHONE_H
#define XGGF_MICROPHONE_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

bool microphone_init(void);
bool microphone_set_rate(uint32_t rate);
size_t microphone_read(int32_t *output, size_t capacity);
void microphone_probe(uint32_t now);
void microphone_stop(void);
bool microphone_active(void);
void microphone_tick(uint32_t now);
bool microphone_take_level(uint32_t *mean, uint32_t *peak);
#endif
