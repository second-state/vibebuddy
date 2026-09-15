#pragma once
#include <stddef.h>
#include "FreeRTOS.h"
typedef struct stub_semaphore *SemaphoreHandle_t;
static inline SemaphoreHandle_t xSemaphoreCreateBinary(void) { return (SemaphoreHandle_t)1; }
static inline BaseType_t xSemaphoreTake(SemaphoreHandle_t handle, TickType_t timeout) { (void)handle; return timeout == 0 ? pdFALSE : pdTRUE; }
static inline void xSemaphoreGiveFromISR(SemaphoreHandle_t handle, BaseType_t *woken) { (void)handle; *woken = pdFALSE; }
