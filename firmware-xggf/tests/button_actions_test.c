#include "button_actions.h"
#include <assert.h>
#include <stdio.h>
#include <string.h>

typedef struct {
    button_event event;
    button_action action;
    button_ui_mode mode;
} expected_transition;

static void assert_state_equal(button_actions_state actual, button_actions_state expected)
{
    assert(actual.mode == expected.mode);
    assert(actual.row == expected.row);
    assert(actual.link_page == expected.link_page);
    assert(actual.last_activity_ms == expected.last_activity_ms);
    assert(actual.recording == expected.recording);
}

static button_transition send(button_actions_state *state, button_event event, uint32_t now)
{
    button_actions_state before = *state;
    button_transition result = button_actions_handle(state, event, now);
    assert_state_equal(result.from, before);
    assert_state_equal(result.to, *state);
    return result;
}

static button_transition record(button_actions_state *state, bool active)
{
    button_actions_state before = *state;
    button_transition result = button_actions_set_recording(state, active);
    assert_state_equal(result.from, before);
    assert_state_equal(result.to, *state);
    return result;
}

static button_transition tick(button_actions_state *state, uint32_t now)
{
    button_actions_state before = *state;
    button_transition result = button_actions_tick(state, now);
    assert_state_equal(result.from, before);
    assert_state_equal(result.to, *state);
    return result;
}

static button_transition reset(button_actions_state *state)
{
    button_actions_state before = *state;
    button_transition result = button_actions_external_reset(state);
    assert_state_equal(result.from, before);
    assert_state_equal(result.to, *state);
    return result;
}

static void assert_transition(button_transition actual, button_action action,
                              button_ui_mode from, button_ui_mode to)
{
    if (actual.action != action || actual.from.mode != from || actual.to.mode != to) {
        fprintf(stderr, "transition mismatch: got %s %u->%u, expected %s %u->%u\n",
                button_action_name(actual.action), actual.from.mode, actual.to.mode,
                button_action_name(action), from, to);
    }
    assert(actual.action == action);
    assert(actual.from.mode == from);
    assert(actual.to.mode == to);
    assert(strcmp(button_action_name(actual.action), "UNKNOWN") != 0);
}

static void enter_menu(button_actions_state *state, uint32_t now)
{
    button_transition opened = send(state, BUTTON_K1_LONG, now);
    assert_transition(opened, BUTTON_ACTION_MENU_OPEN, BUTTON_UI_HOME, BUTTON_UI_MENU);
}

static void select_row(button_actions_state *state, button_menu_row row, uint32_t now)
{
    enter_menu(state, now);
    for (unsigned i = 0; i < (unsigned)row; ++i) {
        button_transition moved = send(state, BUTTON_K1_SHORT, now + i + 1);
        assert(moved.action == BUTTON_ACTION_MENU_NEXT_ROW);
        assert(moved.to.row == (button_menu_row)(i + 1));
    }
}

int main(void)
{
    static const expected_transition home[] = {
        { BUTTON_K0_SHORT, BUTTON_ACTION_MIC_TOGGLE, BUTTON_UI_HOME },
        { BUTTON_K0_LONG, BUTTON_ACTION_IDENTIFY, BUTTON_UI_HOME },
        { BUTTON_K1_SHORT, BUTTON_ACTION_VIEW_NEXT, BUTTON_UI_HOME },
        { BUTTON_K1_LONG, BUTTON_ACTION_MENU_OPEN, BUTTON_UI_MENU },
        { BUTTON_K2_SHORT, BUTTON_ACTION_SOURCE_OPEN, BUTTON_UI_HOME },
        { BUTTON_K2_LONG, BUTTON_ACTION_MUTE_TOGGLE, BUTTON_UI_HOME },
    };
    for (unsigned i = 0; i < sizeof(home) / sizeof(home[0]); ++i) {
        button_actions_state state;
        button_actions_init(&state);
        button_transition result = send(&state, home[i].event, 100 + i);
        assert_transition(result, home[i].action, BUTTON_UI_HOME, home[i].mode);
        assert(state.mode == home[i].mode);
        if (state.mode == BUTTON_UI_MENU) {
        assert(state.row == BUTTON_MENU_MIC && state.link_page == 0);
        assert(result.to.row == BUTTON_MENU_MIC && result.to.link_page == 0);
        }
    }

    static const expected_transition menu[] = {
        { BUTTON_K0_LONG, BUTTON_ACTION_IGNORED_LONG_PRESS, BUTTON_UI_MENU },
        { BUTTON_K1_LONG, BUTTON_ACTION_IGNORED_LONG_PRESS, BUTTON_UI_MENU },
        { BUTTON_K2_LONG, BUTTON_ACTION_MUTE_TOGGLE, BUTTON_UI_MENU },
        { BUTTON_K2_SHORT, BUTTON_ACTION_MENU_CLOSE, BUTTON_UI_HOME },
    };
    for (unsigned i = 0; i < sizeof(menu) / sizeof(menu[0]); ++i) {
        button_actions_state state;
        button_actions_init(&state);
        enter_menu(&state, 200);
        button_transition result = send(&state, menu[i].event, 300 + i);
        assert_transition(result, menu[i].action, BUTTON_UI_MENU, menu[i].mode);
        assert(state.mode == menu[i].mode);
    }

    button_actions_state state;
    button_actions_init(&state);
    enter_menu(&state, 400);
    const button_action row_actions[] = {
        BUTTON_ACTION_MIC_TOGGLE, BUTTON_ACTION_LINK_OPEN, BUTTON_ACTION_BRIGHTNESS_NEXT,
        BUTTON_ACTION_IDENTIFY, BUTTON_ACTION_MUTE_TOGGLE, BUTTON_ACTION_MENU_CLOSE
    };
    const button_ui_mode row_modes[] = {
        BUTTON_UI_MENU, BUTTON_UI_LINK, BUTTON_UI_MENU,
        BUTTON_UI_HOME, BUTTON_UI_MENU, BUTTON_UI_HOME
    };
    for (unsigned row = 0; row < BUTTON_MENU_ROW_COUNT; ++row) {
        button_actions_init(&state);
        select_row(&state, (button_menu_row)row, 500 + row * 20);
        button_transition result = send(&state, BUTTON_K0_SHORT, 510 + row * 20);
        assert_transition(result, row_actions[row], BUTTON_UI_MENU, row_modes[row]);
        assert(state.mode == row_modes[row]);
    }

    button_actions_init(&state);
    enter_menu(&state, 700);
    for (unsigned i = 0; i < 6; ++i) {
        button_transition moved = send(&state, BUTTON_K1_SHORT, 710 + i);
        assert_transition(moved, BUTTON_ACTION_MENU_NEXT_ROW, BUTTON_UI_MENU, BUTTON_UI_MENU);
        assert(moved.to.row == (button_menu_row)((i + 1) % BUTTON_MENU_ROW_COUNT));
    }

    /* LINK pages wrap, and K0/K2 each return to the menu. */
    button_actions_init(&state);
    select_row(&state, BUTTON_MENU_LINK, 800);
    button_transition result = send(&state, BUTTON_K0_SHORT, 810);
    assert_transition(result, BUTTON_ACTION_LINK_OPEN, BUTTON_UI_MENU, BUTTON_UI_LINK);
    for (unsigned i = 1; i <= 4; ++i) {
        result = send(&state, BUTTON_K1_SHORT, 810 + i);
        assert_transition(result, BUTTON_ACTION_LINK_NEXT_PAGE, BUTTON_UI_LINK, BUTTON_UI_LINK);
        assert(result.to.link_page == i % 4);
    }
    result = send(&state, BUTTON_K0_SHORT, 820);
    assert_transition(result, BUTTON_ACTION_LINK_BACK, BUTTON_UI_LINK, BUTTON_UI_MENU);
    assert(result.to.row == BUTTON_MENU_LINK && result.to.link_page == 0);
    result = send(&state, BUTTON_K0_SHORT, 821);
    assert_transition(result, BUTTON_ACTION_LINK_OPEN, BUTTON_UI_MENU, BUTTON_UI_LINK);
    result = send(&state, BUTTON_K2_SHORT, 822);
    assert_transition(result, BUTTON_ACTION_LINK_BACK, BUTTON_UI_LINK, BUTTON_UI_MENU);

    /* Link mode consumes every long press; K2 long retains mute in both menu modes. */
    result = send(&state, BUTTON_K0_LONG, 830);
    assert_transition(result, BUTTON_ACTION_IGNORED_LONG_PRESS, BUTTON_UI_MENU, BUTTON_UI_MENU);
    result = send(&state, BUTTON_K1_LONG, 831);
    assert_transition(result, BUTTON_ACTION_IGNORED_LONG_PRESS, BUTTON_UI_MENU, BUTTON_UI_MENU);
    result = send(&state, BUTTON_K2_LONG, 832);
    assert_transition(result, BUTTON_ACTION_MUTE_TOGGLE, BUTTON_UI_MENU, BUTTON_UI_MENU);
    send(&state, BUTTON_K0_SHORT, 833); /* select LINK */
    result = send(&state, BUTTON_K2_LONG, 834);
    assert_transition(result, BUTTON_ACTION_MUTE_TOGGLE, BUTTON_UI_LINK, BUTTON_UI_LINK);
    result = send(&state, BUTTON_K0_LONG, 835);
    assert_transition(result, BUTTON_ACTION_IGNORED_LONG_PRESS, BUTTON_UI_LINK, BUTTON_UI_LINK);
    result = send(&state, BUTTON_K1_LONG, 836);
    assert_transition(result, BUTTON_ACTION_IGNORED_LONG_PRESS, BUTTON_UI_LINK, BUTTON_UI_LINK);

    /* A recording stop overrides K0 short in every mode without changing UI or timeout state. */
    button_actions_init(&state);
    const button_ui_mode modes[] = { BUTTON_UI_HOME, BUTTON_UI_MENU, BUTTON_UI_LINK };
    for (unsigned i = 0; i < sizeof(modes) / sizeof(modes[0]); ++i) {
        button_actions_init(&state);
        if (modes[i] == BUTTON_UI_MENU) {
            enter_menu(&state, 900 + i);
        } else if (modes[i] == BUTTON_UI_LINK) {
            select_row(&state, BUTTON_MENU_LINK, 910 + i);
            send(&state, BUTTON_K0_SHORT, 920 + i);
        }
        result = record(&state, true);
        assert_transition(result, BUTTON_ACTION_RECORDING_STARTED, modes[i], modes[i]);
        assert(!result.from.recording && result.to.recording);
        assert(state.recording);
        result = send(&state, BUTTON_K0_SHORT, 1000 + i);
        assert_transition(result, BUTTON_ACTION_RECORDING_STOP, modes[i], modes[i]);
        assert(state.mode == modes[i]);
        assert(state.last_activity_ms == (modes[i] == BUTTON_UI_HOME ? 0 : 1000 + i));
        assert(result.from.recording && result.to.recording);
        result = record(&state, false);
        assert_transition(result, BUTTON_ACTION_RECORDING_FINISHED, modes[i], modes[i]);
        assert(!state.recording);
        result = record(&state, false);
        assert_transition(result, BUTTON_ACTION_NONE, modes[i], modes[i]);
    }

    button_actions_init(&state);
    enter_menu(&state, UINT32_MAX - 10000);
    result = tick(&state, 19998);
    assert(result.action == BUTTON_ACTION_NONE && state.mode == BUTTON_UI_MENU);
    result = tick(&state, 19999);
    assert_transition(result, BUTTON_ACTION_MENU_TIMEOUT, BUTTON_UI_MENU, BUTTON_UI_HOME);

    button_actions_init(&state);
    select_row(&state, BUTTON_MENU_LINK, 30000);
    send(&state, BUTTON_K0_SHORT, 30010);
    result = tick(&state, 60009);
    assert(result.action == BUTTON_ACTION_NONE && state.mode == BUTTON_UI_LINK);
    result = tick(&state, 60010);
    assert_transition(result, BUTTON_ACTION_MENU_TIMEOUT, BUTTON_UI_LINK, BUTTON_UI_HOME);

    button_actions_init(&state);
    enter_menu(&state, 1200);
    result = reset(&state);
    assert_transition(result, BUTTON_ACTION_EXTERNAL_RESET, BUTTON_UI_MENU, BUTTON_UI_HOME);
    result = reset(&state);
    assert_transition(result, BUTTON_ACTION_NONE, BUTTON_UI_HOME, BUTTON_UI_HOME);

    button_actions_init(&state);
    enter_menu(&state, 1300);
    send(&state, BUTTON_K1_SHORT, 1301);
    send(&state, BUTTON_K0_SHORT, 1302);
    result = reset(&state);
    assert_transition(result, BUTTON_ACTION_EXTERNAL_RESET, BUTTON_UI_LINK, BUTTON_UI_HOME);

    assert(strcmp(button_action_name(BUTTON_ACTION_MENU_TIMEOUT), "MENU TIMEOUT") == 0);
    return 0;
}
