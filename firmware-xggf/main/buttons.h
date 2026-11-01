#ifndef XGGF_BUTTONS_H
#define XGGF_BUTTONS_H
#include <stdbool.h>
#include <stdint.h>

typedef enum {
    BUTTON_K0_SHORT,
    BUTTON_K0_LONG,
    BUTTON_K1_SHORT,
    BUTTON_K1_LONG,
    BUTTON_K2_SHORT,
    BUTTON_K2_LONG
} button_event;

typedef struct {
    bool pressed;
    uint32_t changed_at;
    uint32_t pressed_at;
    bool long_fired;
} button_state;

typedef struct {
    button_state key[3];
} buttons_state;

void buttons_init(buttons_state *state, const bool pressed[3], uint32_t now);
unsigned buttons_update(buttons_state *state, const bool pressed[3], uint32_t now, button_event events[3]);
const char *button_event_name(button_event event);
#endif
