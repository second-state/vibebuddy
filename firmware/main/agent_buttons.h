#pragma once

#include "esp_err.h"

/// 三个键各管一件事，与模式无关。短按都在松开时触发；K0 与 K1 按住超过
/// 阈值会触发长按，且松开时不再算一次短按。
typedef enum {
  /// K0（BOOT 键，GPIO0）：番茄钟的开始 / 暂停 / 继续。
  AGENT_BUTTON_K0_SHORT,
  /// K0 长按：放弃当前阶段。
  AGENT_BUTTON_K0_LONG,
  /// K1：在值班与番茄钟之间切换。
  AGENT_BUTTON_K1_SHORT,
  /// K1 长按：让小灯灵现在就去休闲。
  AGENT_BUTTON_K1_LONG,
  /// K2：打开当前来源，上报 Mac。
  AGENT_BUTTON_K2_SHORT,
} agent_button_event_t;

typedef void (*agent_button_callback_t)(agent_button_event_t event);

esp_err_t agent_buttons_init(agent_button_callback_t on_event);
void agent_buttons_tick(void);
