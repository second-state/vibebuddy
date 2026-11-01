#include "status.h"
#include "panel.h"
#include <assert.h>
#include <string.h>

static const uint8_t *at(unsigned x, unsigned y, const uint8_t *frame)
{ return frame + (y * PANEL_WIDTH + x) * 3; }

/* The rightmost column carries the connection indicator, so frame equality ignores it. */
static int same_apart_from_indicator(const uint8_t *a, const uint8_t *b)
{
    for (unsigned y = 0; y < PANEL_HEIGHT; ++y)
        for (unsigned x = 0; x + 1 < PANEL_WIDTH; ++x)
            if (memcmp(at(x, y, a), at(x, y, b), 3)) return 0;
    return 1;
}

int main(void)
{
    status_model m; uint8_t offline[PANEL_RGB_BYTES], active[PANEL_RGB_BYTES];
    status_init(&m);
    status_render(&m, 0, offline);
    assert(!m.linked);
    assert(status_event(&m, "task.start", 100));
    status_render(&m, 100, active);
    assert(m.linked && m.state == STATUS_WORKING && memcmp(offline, active, sizeof(active)));
    assert(!status_event(&m, "device.volume", 101));
    assert(m.state == STATUS_WORKING && m.last_message_ms == 100);
    status_tick(&m, 15099); assert(m.linked);
    status_tick(&m, 15100); assert(!m.linked);
    status_render(&m, 15100, active); assert(same_apart_from_indicator(offline, active));
    status_event(&m, "device.heartbeat", 15101); assert(m.linked && m.state == STATUS_READY);
    status_event(&m, "agent.input_required", 16000); assert(m.state == STATUS_INPUT);
    status_event(&m, "task.done", 17000); status_tick(&m, 21999); assert(m.state == STATUS_DONE);
    status_tick(&m, 22000); assert(m.state == STATUS_READY);
    status_event(&m, "task.error", 23000); assert(m.state == STATUS_FAILED);
    status_event(&m, "device.identify", 24000); assert(m.identifying);
    status_tick(&m, 27000); assert(!m.identifying && m.state == STATUS_FAILED);
    status_event(&m, "task.start", UINT32_MAX - 100); status_tick(&m, 14899); assert(!m.linked);
    status_sensors(&m, true, 23, 59, true, 329, 582, 0);
    status_render(&m, 0, offline);
    status_render(&m, 4000, active); assert(memcmp(offline, active, sizeof(active)));
    status_render(&m, 8000, active); assert(memcmp(offline, active, sizeof(active)));
    status_init(&m);
    status_render(&m, 0, offline);
    status_audio_start(&m); status_audio(&m, 1000, 100); assert(m.audio_active && m.audio_level == 1);
    status_render(&m, 100, active); assert(memcmp(offline, active, sizeof(active)));
    memcpy(offline, active, sizeof(active));
    status_audio(&m, 256000, 200); assert(m.audio_level == 9);
    status_render(&m, 200, active); assert(memcmp(offline, active, sizeof(active)));
    status_event(&m, "device.heartbeat", 300); assert(m.audio_active);
    status_event(&m, "task.start", 400); assert(!m.audio_active && !m.audio_session);
    status_audio(&m, 4000, 450); assert(!m.audio_active);
    status_audio_start(&m); status_audio(&m, 4000, 500); status_tick(&m, 999); assert(m.audio_active);
    status_tick(&m, 1000); assert(!m.audio_active && m.audio_session);
    status_audio(&m, 4000, 1100); status_event(&m, "device.identify", 1101);
    assert(!m.audio_active && !m.audio_session);
    status_set_recording(&m, true); assert(m.recording);
    status_render(&m, 1200, active); assert(memcmp(offline, active, sizeof(active)));
    status_event(&m, "task.start", 1201); assert(m.recording);
    status_set_recording(&m, false); assert(!m.recording);

    status_init(&m);
    status_sensors(&m, true, 12, 34, true, 250, 600, 0);
    assert(m.view == STATUS_VIEW_AGENT);
    status_next_view(&m); assert(m.view == STATUS_VIEW_CLOCK);
    status_render(&m, 0, offline); status_render(&m, 4000, active);
    assert(!memcmp(offline, active, sizeof(active)));
    status_next_view(&m); assert(m.view == STATUS_VIEW_ENVIRONMENT);
    status_next_view(&m); assert(m.view == STATUS_VIEW_AGENT);
    status_next_view(&m); status_event(&m, "task.start", 100);
    assert(m.view == STATUS_VIEW_AGENT);
    status_next_view(&m); status_home(&m); assert(m.view == STATUS_VIEW_AGENT);
    status_toggle_mute(&m, 200); assert(m.muted && m.mute_notice);
    status_tick(&m, 2199); assert(m.mute_notice);
    status_tick(&m, 2200); assert(!m.mute_notice && m.muted);
    status_toggle_mute(&m, 2300); assert(!m.muted);
    status_audio_start(&m); status_audio_stop(&m);
    assert(!m.audio_session && !m.audio_active && !m.audio_level);
    status_init(&m);
    status_identify(&m, 2400);
    assert(m.identifying && m.view == STATUS_VIEW_AGENT && !m.linked);
    const bool k1_down[3] = {false, true, false};
    const bool no_long[3] = {false, false, false};
    status_buttons(&m, k1_down, no_long);
    assert(m.button_mask == 2 && !m.long_mask);
    status_render(&m, 2400, active);
    status_buttons(&m, k1_down, k1_down);
    assert(m.long_mask == 2);
    const bool released[3] = {false, false, false};
    status_buttons(&m, released, no_long);
    status_render(&m, 2400, offline);
    assert(!m.button_mask && memcmp(offline, active, sizeof(active)));

    /* Pressed-key dots: 2x2 purple at rows 0-1, columns from the right 18-19 (K0),
       13-14 (K1), 8-9 (K2), drawn over the current view and cleared on release. */
    static const uint8_t purple[3] = {8,0,8};
    const bool all_down[3] = {true, true, true};
    status_buttons(&m, all_down, no_long);
    status_render(&m, 2400, active);
    unsigned label_ink = 0;
    for (unsigned key = 0; key < 3; ++key)
        for (unsigned y = 0; y <= 1; ++y)
            for (unsigned x = 63 - 19 + key * 5; x <= 63 - 18 + key * 5; ++x)
                assert(!memcmp(at(x, y, active), purple, 3));
    for (unsigned y = 0; y < 7; ++y) /* the view behind the dots still renders, no takeover */
        for (unsigned x = 13; x < 44; ++x) {
            const uint8_t *p = at(x, y, active);
            if (p[0] | p[1] | p[2]) ++label_ink;
        }
    assert(label_ink > 20);
    status_buttons(&m, released, no_long);
    status_render(&m, 2400, offline);
    /* releasing drops exactly the dots: the label text can share their columns, so
       compare the held and released frames instead of asserting darkness */
    for (unsigned y = 0; y < PANEL_HEIGHT; ++y)
        for (unsigned x = 0; x + 1 < PANEL_WIDTH; ++x)
            if (!((y == 0 || y == 1) && x >= 44 && x <= 55 && (x - 44) % 5 < 2))
                assert(!memcmp(at(x, y, active), at(x, y, offline), 3));

    /* Connection indicator, rightmost column: rows 0-3 network, rows 4-7 host link. */
    static const uint8_t grey[3] = {3,3,3}, pink[3] = {8,2,3}, tint[3] = {2,8,3};
    status_init(&m);
    status_render(&m, 0, active);
    assert(!memcmp(at(63,0,active), grey, 3) && !memcmp(at(63,3,active), grey, 3));
    assert(!memcmp(at(63,4,active), grey, 3) && !memcmp(at(63,7,active), grey, 3));
    status_network(&m, false, true); status_render(&m, 0, active);
    assert(!memcmp(at(63,0,active), grey, 3));
    status_network(&m, true, false); status_render(&m, 0, active);
    assert(!memcmp(at(63,0,active), pink, 3) && !memcmp(at(63,7,active), grey, 3));
    status_network(&m, true, true); status_render(&m, 0, active);
    assert(!memcmp(at(63,0,active), tint, 3) && !memcmp(at(63,7,active), grey, 3));
    status_event(&m, "device.heartbeat", 100); status_render(&m, 100, active);
    assert(!memcmp(at(63,0,active), tint, 3) && !memcmp(at(63,7,active), tint, 3));
    status_set_recording(&m, true); status_render(&m, 100, active);
    assert(!memcmp(at(63,0,active), tint, 3) && !memcmp(at(63,7,active), tint, 3));
    static const uint8_t red_dot[3] = {8,0,0};
    for (unsigned y = 0; y < 2; ++y)
        for (unsigned x = 61; x <= 62; ++x) assert(!memcmp(at(x,y,active), red_dot, 3));
    status_set_recording(&m, false); status_render(&m, 100, active);
    for (unsigned y = 0; y < 2; ++y)
        for (unsigned x = 61; x <= 62; ++x) assert(!(at(x,y,active)[0] | at(x,y,active)[1] | at(x,y,active)[2]));
    status_tick(&m, 15100); assert(!m.linked); status_render(&m, 15100, active);
    assert(!memcmp(at(63,0,active), tint, 3) && !memcmp(at(63,7,active), pink, 3));
    status_init(&m); status_network(&m, true, true); status_render(&m, 0, active);
    assert(!memcmp(at(63,7,active), grey, 3));
    return 0;
}
