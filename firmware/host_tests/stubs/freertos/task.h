#pragma once
#include "FreeRTOS.h"
/// The preview program sets this clock itself.
extern TickType_t stub_tick_count;
static inline TickType_t xTaskGetTickCount(void) { return stub_tick_count; }
