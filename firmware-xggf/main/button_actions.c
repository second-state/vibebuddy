#include "button_actions.h"

enum { MENU_TIMEOUT_MS = 30000, LINK_PAGE_COUNT = 4 };

static button_transition transition(button_actions_state *state, button_actions_state from,
                                    button_action action)
{
    return (button_transition) {
        .action = action,
        .from = from,
        .to = *state,
    };
}

static void close_menu(button_actions_state *state)
{
    state->mode = BUTTON_UI_HOME;
    state->row = BUTTON_MENU_MIC;
    state->link_page = 0;
}

void button_actions_init(button_actions_state *state)
{
    *state = (button_actions_state) { .mode = BUTTON_UI_HOME, .row = BUTTON_MENU_MIC };
}

button_transition button_actions_handle(button_actions_state *state, button_event event,
                                        uint32_t now)
{
    button_actions_state from = *state;
    button_action action = BUTTON_ACTION_NONE;

    /* Any key activity keeps an open menu alive, even when it is used to stop recording. */
    if (state->mode != BUTTON_UI_HOME) state->last_activity_ms = now;

    /* Recording stop owns K0 short in every UI mode and leaves navigation state intact. */
    if (event == BUTTON_K0_SHORT && state->recording) {
        return transition(state, from, BUTTON_ACTION_RECORDING_STOP);
    }

    if (state->mode == BUTTON_UI_HOME) {
        switch (event) {
            case BUTTON_K0_SHORT: action = state->pomodoro_page ? BUTTON_ACTION_POMODORO_TOGGLE : BUTTON_ACTION_NONE; break;
            case BUTTON_K0_LONG: action = state->recording ? BUTTON_ACTION_IGNORED_LONG_PRESS : BUTTON_ACTION_IDENTIFY; break;
            case BUTTON_K1_SHORT: action = BUTTON_ACTION_VIEW_NEXT; break;
            case BUTTON_K1_LONG:
                state->mode = BUTTON_UI_MENU;
                state->row = BUTTON_MENU_MIC;
                state->link_page = 0;
                state->last_activity_ms = now;
                action = BUTTON_ACTION_MENU_OPEN;
                break;
            case BUTTON_K2_SHORT: action = BUTTON_ACTION_SOURCE_OPEN; break;
            case BUTTON_K2_LONG: action = BUTTON_ACTION_MUTE_TOGGLE; break;
        }
        return transition(state, from, action);
    }

    if (event == BUTTON_K2_LONG) {
        return transition(state, from, BUTTON_ACTION_MUTE_TOGGLE);
    }
    if (event == BUTTON_K0_LONG || event == BUTTON_K1_LONG) {
        return transition(state, from, BUTTON_ACTION_IGNORED_LONG_PRESS);
    }

    if (state->mode == BUTTON_UI_LINK) {
        switch (event) {
            case BUTTON_K0_SHORT:
            case BUTTON_K2_SHORT:
                state->mode = BUTTON_UI_MENU;
                state->link_page = 0;
                action = BUTTON_ACTION_LINK_BACK;
                break;
            case BUTTON_K1_SHORT:
                state->link_page = (state->link_page + 1) % LINK_PAGE_COUNT;
                action = BUTTON_ACTION_LINK_NEXT_PAGE;
                break;
            default: break;
        }
        return transition(state, from, action);
    }

    switch (event) {
        case BUTTON_K0_SHORT:
            switch (state->row) {
                case BUTTON_MENU_MIC: action = BUTTON_ACTION_MIC_TOGGLE; break;
                case BUTTON_MENU_LINK:
                    state->mode = BUTTON_UI_LINK;
                    state->link_page = 0;
                    action = BUTTON_ACTION_LINK_OPEN;
                    break;
                case BUTTON_MENU_BRIGHTNESS: action = BUTTON_ACTION_BRIGHTNESS_NEXT; break;
                case BUTTON_MENU_VOLUME: action = BUTTON_ACTION_VOLUME_NEXT; break;
                case BUTTON_MENU_IDENTIFY:
                    close_menu(state);
                    action = BUTTON_ACTION_IDENTIFY;
                    break;
                case BUTTON_MENU_STOP_FOCUS: action = BUTTON_ACTION_POMODORO_STOP; close_menu(state); break;
                case BUTTON_MENU_MUTE: action = BUTTON_ACTION_MUTE_TOGGLE; break;
                case BUTTON_MENU_CLOSE:
                    close_menu(state);
                    action = BUTTON_ACTION_MENU_CLOSE;
                    break;
                case BUTTON_MENU_ROW_COUNT: break;
            }
            break;
        case BUTTON_K1_SHORT:
            state->row = (button_menu_row)((state->row + 1) % BUTTON_MENU_ROW_COUNT);
            if (state->row == BUTTON_MENU_STOP_FOCUS && !state->pomodoro_active) ++state->row;
            action = BUTTON_ACTION_MENU_NEXT_ROW;
            break;
        case BUTTON_K2_SHORT:
            close_menu(state);
            action = BUTTON_ACTION_MENU_CLOSE;
            break;
        default: break;
    }
    return transition(state, from, action);
}

button_transition button_actions_set_recording(button_actions_state *state, bool recording)
{
    button_actions_state from = *state;
    if (state->recording == recording) return transition(state, from, BUTTON_ACTION_NONE);
    state->recording = recording;
    return transition(state, from, recording ? BUTTON_ACTION_RECORDING_STARTED : BUTTON_ACTION_RECORDING_FINISHED);
}

button_transition button_actions_tick(button_actions_state *state, uint32_t now)
{
    button_actions_state from = *state;
    if (state->mode != BUTTON_UI_HOME &&
        (uint32_t)(now - state->last_activity_ms) >= MENU_TIMEOUT_MS) {
        close_menu(state);
        return transition(state, from, BUTTON_ACTION_MENU_TIMEOUT);
    }
    return transition(state, from, BUTTON_ACTION_NONE);
}

button_transition button_actions_external_reset(button_actions_state *state)
{
    button_actions_state from = *state;
    close_menu(state);
    return transition(state, from,
                      from.mode == BUTTON_UI_HOME ? BUTTON_ACTION_NONE : BUTTON_ACTION_EXTERNAL_RESET);
}

const char *button_action_name(button_action action)
{
    static const char *const names[] = {
        "NONE", "VOLUME NEXT", "POMODORO TOGGLE", "POMODORO STOP", "MIC TOGGLE", "RECORDING STOP", "IDENTIFY", "VIEW NEXT", "SOURCE OPEN",
        "MUTE TOGGLE", "BRIGHTNESS NEXT", "MENU OPEN", "MENU NEXT ROW", "MENU CLOSE",
        "LINK OPEN", "LINK NEXT PAGE", "LINK BACK", "IGNORED LONG PRESS", "MENU TIMEOUT",
        "EXTERNAL RESET", "RECORDING STARTED", "RECORDING FINISHED"
    };
    return (unsigned)action < sizeof(names) / sizeof(names[0]) ? names[action] : "UNKNOWN";
}
