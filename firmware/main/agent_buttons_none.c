#include "agent_buttons.h"

// 面包板尚未连接业务按键；不读取 BOX 的 XL9555，也不把 BOOT 当业务键。
esp_err_t agent_buttons_init(agent_button_callback_t on_event) {
  (void)on_event;
  return ESP_OK;
}

void agent_buttons_tick(void) {}
