#pragma once

#include "esp_err.h"

typedef void (*agent_button_callback_t)(void);

esp_err_t agent_buttons_init(agent_button_callback_t on_k2_pressed);
void agent_buttons_tick(void);
