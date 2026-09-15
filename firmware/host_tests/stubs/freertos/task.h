#pragma once
#include "FreeRTOS.h"
/// 预览程序自己拨这个表。
extern TickType_t stub_tick_count;
static inline TickType_t xTaskGetTickCount(void) { return stub_tick_count; }
