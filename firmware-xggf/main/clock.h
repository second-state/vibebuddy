#ifndef XGGF_CLOCK_H
#define XGGF_CLOCK_H
#include <stdbool.h>
#include <stdint.h>
typedef struct { bool valid; uint32_t day, minute, at_ms; } xggf_clock;
bool clock_sync(xggf_clock *clock, unsigned day, unsigned hour, unsigned minute, uint32_t now);
void clock_tick(xggf_clock *clock, uint32_t now);
#endif
