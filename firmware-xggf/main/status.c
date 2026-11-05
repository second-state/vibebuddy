#include "status.h"
#include "panel.h"
#include "font.h"
#include <stdio.h>
#include <string.h>

void status_init(status_model *m)
{
    memset(m, 0, sizeof(*m));
    m->brightness = 8; m->volume = 50;
    button_actions_init(&m->button_actions);
}

bool status_event(status_model *m, const char *event, uint32_t now)
{
    if (!strcmp(event, "device.hello") || !strcmp(event, "device.heartbeat")) {
        if (!m->linked) { m->state = STATUS_READY; m->done_pending = false; }
        m->linked = true; m->host_seen = true; m->last_message_ms = now; return true;
    }
    if (!strcmp(event, "device.identify")) {
        m->linked = true; m->host_seen = true; m->last_message_ms = now;
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
    m->state = next; m->linked = true; m->host_seen = true; m->last_message_ms = now;
    m->mute_notice = false;
    button_actions_external_reset(&m->button_actions);
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
    m->view = (status_view)((m->view + 1) % 4);
    m->button_actions.pomodoro_page = m->view == STATUS_VIEW_POMODORO;
    m->mute_notice = false;
}

void status_home(status_model *m)
{
    m->view = STATUS_VIEW_AGENT;
    m->button_actions.pomodoro_page = false;
    m->mute_notice = false;
}

void status_identify(status_model *m, uint32_t now)
{
    m->identifying = true;
    m->identify_at_ms = now;
    m->view = STATUS_VIEW_AGENT;
    m->button_actions.pomodoro_page = false;
    m->mute_notice = false;
    button_actions_external_reset(&m->button_actions);
    m->audio_active = false;
    m->audio_session = false;
}

void status_toggle_mute(status_model *m, uint32_t now)
{
    m->muted = !m->muted;
    m->mute_notice = true;
    m->mute_notice_at_ms = now;
}

button_transition status_set_recording(status_model *m, bool recording)
{
    button_transition result = button_actions_set_recording(&m->button_actions, recording);
    if (recording) { m->audio_session = false; m->audio_active = false; }
    return result;
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
    if (!m->host_seen) { m->rtc_valid = rtc_valid; m->rtc_hour = hour; m->rtc_minute = minute; m->rtc_read_ms = now; }
    m->environment_valid = environment_valid; m->temperature_tenths = temperature_tenths; m->humidity_tenths = humidity_tenths;
}

void status_network(status_model *m, bool configured, bool online)
{ m->network_link = !configured ? STATUS_LINK_UNSET : online ? STATUS_LINK_OK : STATUS_LINK_ERROR; }

void status_link_update(status_model *m, const link_status_info *info)
{ m->link_info = *info; }

void status_transport_update(status_model *m, bool usb)
{ m->transport = usb ? STATUS_TRANSPORT_USB : STATUS_TRANSPORT_LAN; }

static void pixel(uint8_t *f, unsigned x, unsigned y, const uint8_t *c)
{ panel_set_pixel(f, x, y, c[0], c[1], c[2]); }

/* The rightmost column is a reserved connection indicator: rows 0-3 the Wi-Fi network, rows 4-7
   the host link (grey unset, pink-red error, tint-green ok). A host that has linked at least once
   reads as error rather than unset after the heartbeat times out. */
static void indicator(uint8_t *f, const status_model *m)
{
    static const uint8_t bar_unset[] = {3,3,3}, bar_error[] = {8,2,3}, bar_ok[] = {2,8,3};
    status_link_state host = m->linked ? STATUS_LINK_OK : m->host_seen ? STATUS_LINK_ERROR : STATUS_LINK_UNSET;
    const uint8_t *network = m->network_link == STATUS_LINK_OK ? bar_ok
        : m->network_link == STATUS_LINK_ERROR ? bar_error : bar_unset;
    const uint8_t *link = host == STATUS_LINK_OK ? (const uint8_t[]){0,6,8} : host == STATUS_LINK_ERROR ? bar_error : bar_unset;
    for (unsigned y = 0; y < 4; ++y) pixel(f, 63, y, network);
    for (unsigned y = 4; y < 8; ++y) pixel(f, 63, y, link);
}

/* 5x7 petabyt glyphs in rows 0-6, six pixels per character: the area right of the
   buddy holds eight characters, and longer labels clip short of the indicator column. */
static void text(uint8_t *f, const char *s, const uint8_t *c)
{
    unsigned x = 13;
    while (*s && x + XGGF_FONT_WIDTH - 1 < PANEL_WIDTH - 1) {
        const xggf_glyph *g = xggf_font_glyph(*s);
        if (g)
            for (unsigned y = 0; y < XGGF_FONT_HEIGHT; ++y)
                for (unsigned col = 0; col < XGGF_FONT_WIDTH; ++col)
                    if (g->rows[y] & (0x10u >> col)) pixel(f, x + col, y, c);
        x += XGGF_FONT_WIDTH + 1; ++s;
    }
}

/* Pressed-key feedback: a temporary 2x2 purple dot per held key, rows 0-1 at columns
   from the right 17-18 (K0), 12-13 (K1), 8-9 (K2), drawn over the current view. */
static void button_dots(uint8_t *f, const status_model *m)
{
    static const uint8_t purple[] = {8,0,8};
    static const unsigned lefts[3] = {45, 50, 54};
    for (unsigned key = 0; key < 3; ++key)
        if (m->button_mask & (1u << key)) {
            unsigned left = lefts[key];
            for (unsigned x = left; x <= left + 1; ++x)
                for (unsigned y = 0; y <= 1; ++y) pixel(f, x, y, purple);
        }
}

static void button_feedback(uint8_t *f, const status_model *m)
{
    static const uint8_t purple[] = {8,0,8}, dark[] = {0,0,0};
    button_dots(f, m);
    if (m->long_mask) {
        for (unsigned y = 0; y < XGGF_FONT_HEIGHT; ++y)
            for (unsigned x = 13; x < 60; ++x) pixel(f, x, y, dark);
        text(f, "LONG", purple);
        button_dots(f, m);
    }
}

void status_render(const status_model *m, uint32_t now, uint8_t *f)
{
    static const uint8_t gray[] = {3,3,3}, cyan[] = {0,6,8}, amber[] = {8,4,0},
        green[] = {0,8,0}, red[] = {8,0,0}, white[] = {8,8,8};
    memset(f, 0, PANEL_RGB_BYTES);
    indicator(f, m);
    /* A steady 2x2 red dot in the top-right corner marks active voice recording in every
       view; the corner beside the bar stays clear of the button dots and label text. */
    if (m->state == STATUS_INPUT && (now / 500) % 2 == 0)
        for (unsigned x = 61; x <= 62; ++x) { pixel(f,x,6,amber); pixel(f,x,7,amber); }
    if (m->button_actions.recording) for (unsigned x = 61; x <= 62; ++x) { pixel(f,x,0,red); pixel(f,x,1,red); }
    if (m->button_actions.recording) {
        for (unsigned x = 2; x <= 8; ++x) { pixel(f,x,0,red); pixel(f,x,7,red); }
        for (unsigned y = 1; y < 7; ++y) { pixel(f,1,y,red); pixel(f,9,y,red); }
        pixel(f,3,2,red); pixel(f,7,2,red); pixel(f,5,5,red);
        if ((now / 500) % 2) pixel(f,12,1,red);
        text(f,"REC",red);
        button_feedback(f, m);
        return;
    }
    if (m->button_actions.mode != BUTTON_UI_HOME) {
        const char *label = "LINK";
        char value[16];
        if (m->button_actions.mode == BUTTON_UI_LINK) {
            switch (m->button_actions.link_page % 4) {
                case 0:
                    label = !m->linked ? "OFFLINE" : m->transport == STATUS_TRANSPORT_USB ? "LINK USB" : "LINK LAN";
                    break;
                case 1:
                    switch (m->link_info.wifi_state) {
                        case LINK_WIFI_NOT_SET: label = "NOT SET"; break;
                        case LINK_WIFI_CONNECTING: label = "JOINING"; break;
                        case LINK_WIFI_JOINED: label = m->link_info.ssid; break;
                        case LINK_WIFI_FAILED: label = "WIFI FAIL"; break;
                        case LINK_WIFI_DISCONNECTED: label = "WIFI DOWN"; break;
                    }
                    break;
                case 2: label = m->link_info.ip[0] ? m->link_info.ip : "NO IP"; break;
                case 3:
                    snprintf(value, sizeof(value), "PAIRS %u", m->link_info.paired_count);
                    label = value;
                    break;
            }
        } else switch (m->button_actions.row) {
            case BUTTON_MENU_MIC: label = m->audio_session ? "MIC ON" : "MIC OFF"; break;
            case BUTTON_MENU_LINK: label = "LINK"; break;
            case BUTTON_MENU_BRIGHTNESS:
                snprintf(value, sizeof(value), "BRIGHT %u", m->brightness); label = value; break;
            case BUTTON_MENU_VOLUME: snprintf(value,sizeof(value),"VOL %u",m->volume); label=value; break;
            case BUTTON_MENU_STOP_FOCUS: label = "STOP FOC"; break;
            case BUTTON_MENU_IDENTIFY: label = "IDENTIFY"; break;
            case BUTTON_MENU_MUTE: label = m->muted ? "MUTE ON" : "MUTE OFF"; break;
            case BUTTON_MENU_CLOSE: label = "CLOSE"; break;
            case BUTTON_MENU_ROW_COUNT: break;
        }
        for (unsigned x = 2; x <= 8; ++x) { pixel(f,x,0,cyan); pixel(f,x,7,cyan); }
        for (unsigned y = 1; y < 7; ++y) { pixel(f,1,y,cyan); pixel(f,9,y,cyan); }
        pixel(f,3,2,cyan); pixel(f,7,2,cyan);
        text(f,label,cyan);
        button_feedback(f, m);
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
        button_feedback(f, m);
        return;
    }
    if (m->view == STATUS_VIEW_POMODORO) {
        const pomodoro_view *v = &m->pomodoro;
        const uint8_t *c = v->phase ? green : v->run ? red : cyan;
        uint8_t pulse[3];
        uint32_t alarm_age=(uint32_t)(now-m->pomodoro_alarm_at_ms);
        if(m->pomodoro_alarm && alarm_age>=2000) {
            unsigned t=alarm_age%2000;
            unsigned brightness=2+(t<1000?t:2000-t)*6/1000;
            for(unsigned i=0;i<3;++i) pulse[i]=c[i]*brightness/8;
            c=pulse;
        }
        char timer[16];
        unsigned seconds = (v->remaining_ms + 999) / 1000;
        snprintf(timer, sizeof(timer), "%02u:%02u", seconds / 60, seconds % 60);
        if (v->run != 2 || (now / 500) % 2 == 0) text(f, timer, c);
        unsigned filled = v->total_ms ? (uint64_t)(v->total_ms - v->remaining_ms) * 46 / v->total_ms : 0;
        if (m->pomodoro_alarm) filled = 46;
        for (unsigned x = 13; x < 59; ++x)
            pixel(f, x, 7, x < 13 + filled && (!m->pomodoro_alarm || alarm_age>=2000 || (now / 500) % 2 == 0) ? c : gray);
        snprintf(timer, sizeof(timer), "%u", (unsigned)(v->completed > 99 ? 99 : v->completed));
        for(unsigned digit=0;timer[digit];++digit) {
            const xggf_glyph *g = xggf_font_glyph(timer[digit]);
            if (g) for (unsigned y=0;y<7;++y) for(unsigned x=0;x<5;++x)
                if (g->rows[y] & (0x10u >> x)) pixel(f,digit*6+x,y,c);
        }
        button_feedback(f,m);
        return;
    }
    const uint8_t *color = gray; const char *label = "OFFLINE"; char sensor_label[32];
    if (m->view == STATUS_VIEW_CLOCK) {
        color = cyan;
        if (m->rtc_valid) {
            unsigned minute = (m->rtc_hour * 60 + m->rtc_minute + (uint32_t)(now - m->rtc_read_ms) / 60000) % 1440;
            snprintf(sensor_label, sizeof(sensor_label), "%02u:%02u", minute / 60, minute % 60);
            label = sensor_label;
        } else label = "NO TIME";
    } else if (m->view == STATUS_VIEW_ENVIRONMENT) {
        color = cyan;
        if (m->environment_valid) {
            snprintf(sensor_label, sizeof(sensor_label), "%dC %u%%", m->temperature_tenths / 10, m->humidity_tenths / 10);
            label = sensor_label;
        } else label = "NO ENV";
    } else {
        if (!m->linked) {
            if (m->rtc_valid) {
                unsigned minute = (m->rtc_hour * 60 + m->rtc_minute + (uint32_t)(now - m->rtc_read_ms) / 60000) % 1440;
                snprintf(sensor_label,sizeof(sensor_label),"%02u:%02u",minute/60,minute%60);
                label = sensor_label;
            } else label = "NO TIME";
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
    button_feedback(f, m);
}
