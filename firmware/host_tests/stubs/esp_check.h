#pragma once
#include "esp_err.h"
#define ESP_RETURN_ON_ERROR(expression, tag, ...)                 \
  do {                                                            \
    esp_err_t stub_result = (expression);                         \
    if (stub_result != ESP_OK) {                                  \
      return stub_result;                                         \
    }                                                             \
  } while (0)
