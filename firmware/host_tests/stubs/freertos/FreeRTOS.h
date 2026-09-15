#pragma once
#include <stdint.h>
typedef uint32_t TickType_t;
typedef int BaseType_t;
#define pdTRUE 1
#define pdFALSE 0
#define configTICK_RATE_HZ 100u
#define portTICK_PERIOD_MS ((TickType_t)1000 / configTICK_RATE_HZ)
#define pdMS_TO_TICKS(ms) ((TickType_t)((uint64_t)(ms) * configTICK_RATE_HZ / 1000))
#define pdTICKS_TO_MS(ticks) ((TickType_t)((uint64_t)(ticks) * 1000 / configTICK_RATE_HZ))
