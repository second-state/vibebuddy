#include "status.h"
#include "panel.h"
#include <stdio.h>
#include <string.h>

void status_init(status_model *m) { memset(m, 0, sizeof(*m)); }

bool status_event(status_model *m, const char *event, uint32_t now)
{
    if (!strcmp(event, "device.hello") || !strcmp(event, "device.heartbeat")) {
        if (!m->linked) { m->state = STATUS_READY; m->done_pending = false; }
        m->linked = true; m->last_message_ms = now; return true;
    }
    if (!strcmp(event, "device.identify")) {
        m->linked = true; m->last_message_ms = now;
        status_identify(m, now);
        return true;
    }
    status_kind next;
    if (!strcmp(event, "task.start")) next = STATUS_WORKING;
    else if (!strcmp(event, "agent.idle")) next = STATUS_READY;
    else if (!strcmp(event, "agent.input_required")) next = STATUS_INPUT;
    else if (!strcmp(event, "task.done")) next = STATUS_DONE;
    else if (!strcmp(event, "task.error") || !strcmp(event, "agent.blocked")) next = STATUS_FAILED;
    else return false;
    m->state = next; m->linked = true; m->last_message_ms = now;
    m->view = STATUS_VIEW_AGENT; m->mute_notice = false;
    m->done_pending = next == STATUS_DONE; m->done_at_ms = now;
    m->audio_active = false; m->audio_session = false;
    return true;
}

void status_tick(status_model *m, uint32_t now)
{
    if (m->linked && (uint32_t)(now - m->last_message_ms) >= 15000) m->linked = false;
    if (m->done_pending && (uint32_t)(now - m->done_at_ms) >= 5000) { m->state = STATUS_READY; m->done_pending = false; }
    if (m->identifying && (uint32_t)(now - m->identify_at_ms) >= 3000) m->identifying = false;
    if (m->audio_active && (uint32_t)(now - m->audio_at_ms) >= 500) m->audio_active = false;
    if (m->mute_notice && (uint32_t)(now - m->mute_notice_at_ms) >= 2000) m->mute_notice = false;
}

void status_buttons(status_model *m, const bool pressed[3], const bool long_fired[3])
{
    m->button_mask = (pressed[0] ? 1u : 0u) | (pressed[1] ? 2u : 0u) | (pressed[2] ? 4u : 0u);
    m->long_mask = (long_fired[0] ? 1u : 0u) | (long_fired[1] ? 2u : 0u) | (long_fired[2] ? 4u : 0u);
}

void status_next_view(status_model *m)
{
    m->view = (status_view)((m->view + 1) % 3);
    m->mute_notice = false;
}

void status_home(status_model *m)
{
    m->view = STATUS_VIEW_AGENT;
    m->mute_notice = false;
}

void status_identify(status_model *m, uint32_t now)
{
    m->identifying = true;
    m->identify_at_ms = now;
    m->view = STATUS_VIEW_AGENT;
    m->mute_notice = false;
    m->audio_active = false;
    m->audio_session = false;
}

void status_toggle_mute(status_model *m, uint32_t now)
{
    m->muted = !m->muted;
    m->view = STATUS_VIEW_AGENT;
    m->mute_notice = true;
    m->mute_notice_at_ms = now;
}

void status_set_recording(status_model *m, bool recording)
{
    m->recording = recording;
    if (recording) { m->audio_session = false; m->audio_active = false; }
}

void status_audio_start(status_model *m)
{
    m->audio_session = true; m->audio_active = false; m->audio_level = 0;
}

void status_audio_stop(status_model *m)
{
    m->audio_session = false; m->audio_active = false; m->audio_level = 0;
}

void status_audio(status_model *m, uint32_t mean, uint32_t now)
{
    if (!m->audio_session) return;
    unsigned level = 0;
    uint32_t threshold = 1000;
    while (level < 10 && mean >= threshold) { ++level; threshold *= 2; }
    m->audio_active = true; m->audio_level = level; m->audio_at_ms = now;
}

void status_sensors(status_model *m, bool rtc_valid, unsigned hour, unsigned minute,
                    bool environment_valid, int temperature_tenths, unsigned humidity_tenths, uint32_t now)
{
    m->rtc_valid = rtc_valid; m->rtc_hour = hour; m->rtc_minute = minute; m->rtc_read_ms = now;
    m->environment_valid = environment_valid; m->temperature_tenths = temperature_tenths; m->humidity_tenths = humidity_tenths;
}

/* 3x5 uppercase glyphs, rows packed in the low three bits. */
static const struct { char c; uint8_t rows[5]; } glyphs[] = {
    {'A',{2,5,7,5,5}}, {'D',{6,5,5,5,6}}, {'E',{7,4,6,4,7}}, {'F',{7,4,6,4,4}},
    {'G',{3,4,5,5,3}}, {'I',{7,2,2,2,7}}, {'K',{5,5,6,5,5}}, {'L',{4,4,4,4,7}},
    {'M',{5,7,7,5,5}}, {'N',{5,7,7,7,5}}, {'O',{2,5,5,5,2}}, {'P',{6,5,6,4,4}},
    {'R',{6,5,6,5,5}}, {'T',{7,2,2,2,2}}, {'U',{5,5,5,5,7}}, {'V',{5,5,5,5,2}},
    {'W',{5,5,7,7,5}}, {'Y',{5,5,2,2,2}},
    {'0',{7,5,5,5,7}}, {'1',{2,6,2,2,7}}, {'2',{6,1,2,4,7}}, {'3',{6,1,2,1,6}},
    {'4',{5,5,7,1,1}}, {'5',{7,4,6,1,6}}, {'6',{3,4,7,5,7}}, {'7',{7,1,2,2,2}},
    {'8',{7,5,7,5,7}}, {'9',{7,5,7,1,6}}, {'C',{3,4,4,4,3}},
    {':',{0,2,0,2,0}}, {'%',{5,1,2,4,5}}, {'-',{0,0,7,0,0}}
};

static void pixel(uint8_t *f, unsigned x, unsigned y, const uint8_t *c)
{ panel_set_pixel(f, x, y, c[0], c[1], c[2]); }

static void text(uint8_t *f, const char *s, const uint8_t *c)
{
    unsigned x = 13;
    while (*s && x + 2 < PANEL_WIDTH) {
        for (unsigned g = 0; g < sizeof(glyphs)/sizeof(glyphs[0]); ++g) if (glyphs[g].c == *s) {
            for (unsigned y = 0; y < 5; ++y) for (unsigned col = 0; col < 3; ++col)
                if (glyphs[g].rows[y] & (4u >> col)) pixel(f, x + col, y + 1, c);
            break;
        }
        x += 4; ++s;
    }
}

void status_render(const status_model *m, uint32_t now, uint8_t *f)
{
    static const uint8_t gray[] = {3,3,3}, cyan[] = {0,6,8}, amber[] = {8,4,0},
        green[] = {0,8,0}, red[] = {8,0,0}, white[] = {8,8,8};
    memset(f, 0, PANEL_RGB_BYTES);
    if (m->button_mask) {
        unsigned key = m->button_mask & 1 ? 0 : m->button_mask & 2 ? 1 : 2;
        char label[16];
        snprintf(label, sizeof(label), "K%u %s", key, (m->long_mask & (1u << key)) ? "LONG" : "DOWN");
        text(f, label, white);
        return;
    }
    if (m->recording) {
        for (unsigned x = 2; x <= 8; ++x) { pixel(f,x,0,red); pixel(f,x,7,red); }
        for (unsigned y = 1; y < 7; ++y) { pixel(f,1,y,red); pixel(f,9,y,red); }
        pixel(f,3,2,red); pixel(f,7,2,red); pixel(f,5,5,red);
        if ((now / 500) % 2) pixel(f,12,1,red);
        text(f,"REC",red);
        return;
    }
    if (m->audio_session) {
        const uint8_t *color = m->audio_level >= 8 ? amber : cyan;
        for (unsigned x = 2; x <= 8; ++x) { pixel(f,x,0,color); pixel(f,x,7,color); }
        for (unsigned y = 1; y < 7; ++y) { pixel(f,1,y,color); pixel(f,9,y,color); }
        pixel(f,3,2,color); pixel(f,7,2,color);
        for (unsigned x = 4; x <= 6; ++x) pixel(f,x,m->audio_level >= 5 ? 6 : 5,color);
        for (unsigned bar = 0; bar < m->audio_level; ++bar) {
            unsigned height = 2 + bar * 6 / 9;
            for (unsigned x = 13 + bar * 5; x < 16 + bar * 5; ++x)
                for (unsigned y = 8 - height; y < 8; ++y) pixel(f,x,y,color);
        }
        return;
    }
    const uint8_t *color = gray; const char *label = "OFFLINE"; char sensor_label[32];
    if (m->view == STATUS_VIEW_CLOCK) {
        color = cyan;
        if (m->rtc_valid) {
            unsigned minute = (m->rtc_hour * 60 + m->rtc_minute + (uint32_t)(now - m->rtc_read_ms) / 60000) % 1440;
            snprintf(sensor_label, sizeof(sensor_label), "%02u:%02u", minute / 60, minute % 60);
            label = sensor_label;
        } else label = "NO RTC";
    } else if (m->view == STATUS_VIEW_ENVIRONMENT) {
        color = cyan;
        if (m->environment_valid) {
            snprintf(sensor_label, sizeof(sensor_label), "%dC %u%%", m->temperature_tenths / 10, m->humidity_tenths / 10);
            label = sensor_label;
        } else label = "NO ENV";
    } else {
        if (!m->linked && m->rtc_valid && (now / 4000) % 3 == 1) {
            unsigned minute = (m->rtc_hour * 60 + m->rtc_minute + (uint32_t)(now - m->rtc_read_ms) / 60000) % 1440;
            snprintf(sensor_label, sizeof(sensor_label), "%02u:%02u", minute / 60, minute % 60);
            label = sensor_label;
        } else if (!m->linked && m->environment_valid && (now / 4000) % 3 == 2) {
            snprintf(sensor_label, sizeof(sensor_label), "%dC %u%%", m->temperature_tenths / 10, m->humidity_tenths / 10);
            label = sensor_label;
        }
        if (m->linked) switch (m->state) {
            case STATUS_READY: color = cyan; label = "READY"; break;
            case STATUS_WORKING: color = cyan; label = "WORKING"; break;
            case STATUS_INPUT: color = amber; label = "INPUT"; break;
            case STATUS_DONE: color = green; label = "DONE"; break;
            case STATUS_FAILED: color = red; label = "FAILED"; break;
        }
        if (m->mute_notice) {
            color = m->muted ? amber : cyan;
            label = m->muted ? "MUTE ON" : "MUTE OFF";
        }
    }
    if (m->identifying && (now / 250) % 2) color = white;
    for (unsigned x = 2; x <= 8; ++x) { pixel(f,x,0,color); pixel(f,x,7,color); }
    for (unsigned y = 1; y < 7; ++y) { pixel(f,1,y,color); pixel(f,9,y,color); }
    if (m->linked) {
        pixel(f,3,2,color); pixel(f,7,2,color);
        if (m->state == STATUS_INPUT || m->state == STATUS_FAILED) pixel(f,5,5,color);
        else { pixel(f,4,5,color); pixel(f,5,6,color); pixel(f,6,5,color); }
    } else {
        for (unsigned x = 3; x <= 7; ++x) pixel(f,x,3,color);
    }
    if (m->view == STATUS_VIEW_AGENT && m->linked && m->state == STATUS_WORKING && (now / 400) % 2) pixel(f,5,3,color);
    text(f,label,color);
}
