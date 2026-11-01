#include "buttons.h"
#include <assert.h>
#include <string.h>

static button_event one(buttons_state *state, const bool levels[3], uint32_t now)
{
    button_event events[3];
    unsigned count = buttons_update(state, levels, now, events);
    assert(count <= 1);
    return count ? events[0] : (button_event)-1;
}

int main(void)
{
    buttons_state state;
    const bool released[3] = {false, false, false};
    const bool k0[3] = {true, false, false};
    const bool k1[3] = {false, true, false};
    const bool k2[3] = {false, false, true};
    buttons_init(&state, released, 0);

    assert(one(&state, k0, 100) == (button_event)-1);
    assert(one(&state, released, 160) == BUTTON_K0_SHORT);
    assert(one(&state, k1, 200) == (button_event)-1);
    assert(one(&state, released, 240) == BUTTON_K1_SHORT);
    assert(one(&state, k2, 300) == (button_event)-1);
    assert(one(&state, released, 400) == BUTTON_K2_SHORT);

    assert(one(&state, k0, 500) == (button_event)-1);
    assert(one(&state, released, 520) == (button_event)-1);
    assert(one(&state, k0, 530) == (button_event)-1);
    assert(one(&state, released, 600) == BUTTON_K0_SHORT);

    assert(one(&state, k1, 700) == (button_event)-1);
    assert(one(&state, k1, 1699) == (button_event)-1);
    assert(one(&state, k1, 1700) == BUTTON_K1_LONG);
    assert(one(&state, k1, 2500) == (button_event)-1);
    assert(one(&state, released, 2600) == (button_event)-1);

    assert(one(&state, k2, 3000) == (button_event)-1);
    assert(one(&state, k2, 4000) == BUTTON_K2_LONG);
    assert(one(&state, released, 4100) == (button_event)-1);
    assert(!strcmp(button_event_name(BUTTON_K0_SHORT), "K0 SHORT"));
    assert(!strcmp(button_event_name(BUTTON_K2_LONG), "K2 LONG"));

    buttons_init(&state, released, UINT32_MAX - 500);
    assert(one(&state, k0, UINT32_MAX - 400) == (button_event)-1);
    assert(one(&state, k0, 599) == BUTTON_K0_LONG);
    return 0;
}
