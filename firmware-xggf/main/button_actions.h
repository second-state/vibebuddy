#ifndef XGGF_BUTTON_ACTIONS_H
#define XGGF_BUTTON_ACTIONS_H

#include <stdbool.h>
#include <stdint.h>
#include "buttons.h"

typedef enum {
    BUTTON_UI_HOME,
    BUTTON_UI_MENU,
    BUTTON_UI_LINK
} button_ui_mode;

typedef enum {
    BUTTON_MENU_MIC,
    BUTTON_MENU_LINK,
    BUTTON_MENU_BRIGHTNESS,
    BUTTON_MENU_VOLUME,
    BUTTON_MENU_IDENTIFY,
    BUTTON_MENU_STOP_FOCUS,
    BUTTON_MENU_MUTE,
    BUTTON_MENU_CLOSE,
    BUTTON_MENU_ROW_COUNT
} button_menu_row;

/* Each gesture has one semantic result; menu navigation is represented separately from effects. */
typedef enum {
    BUTTON_ACTION_NONE,
    BUTTON_ACTION_VOLUME_NEXT,
    BUTTON_ACTION_POMODORO_TOGGLE,
    BUTTON_ACTION_POMODORO_STOP,
    BUTTON_ACTION_MIC_TOGGLE,
    BUTTON_ACTION_RECORDING_STOP,
    BUTTON_ACTION_IDENTIFY,
    BUTTON_ACTION_VIEW_NEXT,
    BUTTON_ACTION_SOURCE_OPEN,
    BUTTON_ACTION_MUTE_TOGGLE,
    BUTTON_ACTION_BRIGHTNESS_NEXT,
    BUTTON_ACTION_MENU_OPEN,
    BUTTON_ACTION_MENU_NEXT_ROW,
    BUTTON_ACTION_MENU_CLOSE,
    BUTTON_ACTION_LINK_OPEN,
    BUTTON_ACTION_LINK_NEXT_PAGE,
    BUTTON_ACTION_LINK_BACK,
    BUTTON_ACTION_IGNORED_LONG_PRESS,
    BUTTON_ACTION_MENU_TIMEOUT,
    BUTTON_ACTION_EXTERNAL_RESET,
    BUTTON_ACTION_RECORDING_STARTED,
    BUTTON_ACTION_RECORDING_FINISHED
} button_action;

typedef struct {
    button_ui_mode mode;
    button_menu_row row;
    unsigned link_page;
    uint32_t last_activity_ms;
    bool recording;
    bool pomodoro_page, pomodoro_active;
} button_actions_state;

typedef struct {
    button_action action;
    button_actions_state from;
    button_actions_state to;
} button_transition;

void button_actions_init(button_actions_state *state);
button_transition button_actions_handle(button_actions_state *state, button_event event,
                                        uint32_t now);
button_transition button_actions_set_recording(button_actions_state *state, bool recording);
button_transition button_actions_tick(button_actions_state *state, uint32_t now);
button_transition button_actions_external_reset(button_actions_state *state);
const char *button_action_name(button_action action);

#endif
