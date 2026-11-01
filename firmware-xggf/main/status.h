#ifndef XGGF_STATUS_H
#define XGGF_STATUS_H
#include <stdbool.h>
#include <stdint.h>

typedef enum { STATUS_READY, STATUS_WORKING, STATUS_INPUT, STATUS_DONE, STATUS_FAILED } status_kind;
typedef enum { STATUS_VIEW_AGENT, STATUS_VIEW_CLOCK, STATUS_VIEW_ENVIRONMENT } status_view;
typedef struct {
    status_kind state;
    status_view view;
    uint32_t last_message_ms;
    uint32_t done_at_ms;
    uint32_t identify_at_ms;
    uint32_t audio_at_ms;
    bool linked;
    bool done_pending;
    bool identifying;
    bool audio_active;
    bool audio_session;
    bool recording;
    bool muted;
    bool mute_notice;
    uint32_t mute_notice_at_ms;
    unsigned button_mask;
    unsigned long_mask;
    unsigned audio_level;
    bool rtc_valid, environment_valid;
    unsigned rtc_hour, rtc_minute, humidity_tenths;
    int temperature_tenths;
    uint32_t rtc_read_ms;
} status_model;

void status_init(status_model *model);
bool status_event(status_model *model, const char *event, uint32_t now);
void status_tick(status_model *model, uint32_t now);
void status_buttons(status_model *model, const bool pressed[3], const bool long_fired[3]);
void status_next_view(status_model *model);
void status_home(status_model *model);
void status_identify(status_model *model, uint32_t now);
void status_toggle_mute(status_model *model, uint32_t now);
void status_set_recording(status_model *model, bool recording);
void status_audio_start(status_model *model);
void status_audio_stop(status_model *model);
void status_audio(status_model *model, uint32_t mean, uint32_t now);
void status_sensors(status_model *model, bool rtc_valid, unsigned hour, unsigned minute,
                    bool environment_valid, int temperature_tenths, unsigned humidity_tenths, uint32_t now);
void status_render(const status_model *model, uint32_t now, uint8_t *frame);
#endif
