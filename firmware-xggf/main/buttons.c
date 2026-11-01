#include "buttons.h"

#define DEBOUNCE_MS 40
#define LONG_PRESS_MS 1000

static button_event update_key(button_state *key, bool pressed, uint32_t now, int index)
{
    button_event none = (button_event)-1;
    if (pressed != key->pressed) {
        if ((int32_t)(now - key->changed_at) < DEBOUNCE_MS) return none;
        key->pressed = pressed;
        key->changed_at = now;
        if (pressed) {
            key->pressed_at = now;
            key->long_fired = false;
            return none;
        }
        if (key->long_fired) return none;
        return (button_event)(BUTTON_K0_SHORT + index * 2);
    }
    if (pressed && !key->long_fired && (int32_t)(now - key->pressed_at) >= LONG_PRESS_MS) {
        key->long_fired = true;
        return (button_event)(BUTTON_K0_LONG + index * 2);
    }
    return none;
}

void buttons_init(buttons_state *state, const bool pressed[3], uint32_t now)
{
    for (unsigned i = 0; i < 3; ++i) {
        state->key[i].pressed = pressed[i];
        state->key[i].changed_at = now;
        state->key[i].pressed_at = now;
        state->key[i].long_fired = false;
    }
}

unsigned buttons_update(buttons_state *state, const bool pressed[3], uint32_t now, button_event events[3])
{
    unsigned count = 0;
    for (unsigned i = 0; i < 3; ++i) {
        button_event event = update_key(&state->key[i], pressed[i], now, (int)i);
        if ((int)event >= 0) events[count++] = event;
    }
    return count;
}

const char *button_event_name(button_event event)
{
    static const char *names[] = { "K0 SHORT", "K0 LONG", "K1 SHORT", "K1 LONG", "K2 SHORT", "K2 LONG" };
    return (unsigned)event < sizeof(names) / sizeof(names[0]) ? names[event] : "UNKNOWN";
}
