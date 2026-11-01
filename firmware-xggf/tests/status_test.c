#include "status.h"
#include "panel.h"
#include <assert.h>
#include <string.h>

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
    status_render(&m, 15100, active); assert(!memcmp(offline, active, sizeof(active)));
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
    return 0;
}
