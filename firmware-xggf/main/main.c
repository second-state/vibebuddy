#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "cJSON.h"
#include "driver/gpio.h"
#include "driver/spi_master.h"
#include "driver/i2c.h"
#include "driver/usb_serial_jtag.h"
#include "driver/usb_serial_jtag_vfs.h"
#include "esp_heap_caps.h"
#include "esp_log.h"
#include "esp_system.h"
#include "esp_timer.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "audio.h"
#include "buttons.h"
#include "buzzer.h"
#include "microphone.h"
#include "panel.h"
#include "settings.h"
#include "status.h"
#include "clock.h"
#include "rtc.h"
#include "wifi_link.h"

/* The verified stock SPI setup uses GPIO 7 for MOSI and GPIO 21 for SCLK. */
enum { PANEL_DATA_GPIO = 7, PANEL_SPI_HZ = 3200000, MAX_LINE = 4608 };
static uint8_t frame[PANEL_RGB_BYTES];
static char line[MAX_LINE + 1];
static bool sensor_probed;
static bool rtc_found, environment_found;
static bool rtc_valid, environment_valid;
static unsigned rtc_hour, rtc_minute, humidity_tenths;
static int temperature_tenths;

static uint32_t now_ms(void) { return (uint32_t)(esp_timer_get_time() / 1000); }

/* A network-armed recording waits out a brief peerless gap in the audio ring; this bounds the wait
 * so the END still goes out on time even if the peer never returns. */
enum { AUDIO_HOLD_MARGIN_MS = 250 };
static bool audio_armed_from_usb = true;
static uint32_t audio_hold_deadline;

static bool audio_output(void *context, const char *output)
{
    (void)context;
    bool sent = wifi_link_deliver(output);
    puts(output);
    if (!strncmp(output, "AUDIO BEGIN ", 12)) wifi_link_diag_mark();
    if (!strncmp(output, "AUDIO END ", 10) && sent) wifi_link_diagnose();
    /* A USB-armed recording counts the console as its destination; a network-armed one only the link. */
    return audio_armed_from_usb || sent;
}

/* Runs from inside a blocked send's wait, so a closed TCP window cannot starve the microphone's
 * DMA queue. Static storage: it fires from as deep as the sealed-line send path. */
static void drain_microphone(void)
{
    static int32_t samples[512];
    size_t count = microphone_read(samples, 512);
    if (count) audio_push(samples, count, now_ms());
}

static void announce(void)
{
    /* Board first: do not expose a build before its hardware identity. */
    puts("BOARD xggf-64x8");
    puts("DISPLAY READY BUILD xggf-experimental");
    puts("FIRMWARE VERSION 0.1.0");
    puts("MODE duty");
    wifi_link_report("BOARD xggf-64x8");
    wifi_link_report("DISPLAY READY BUILD xggf-experimental");
    wifi_link_report("FIRMWARE VERSION 0.1.0");
    wifi_link_report("MODE duty");
    wifi_link_announce();
    if (sensor_probed) {
        printf("SENSOR I2C 32 %s\n", rtc_found ? "FOUND" : "NOT_FOUND");
        printf("SENSOR I2C 38 %s\n", environment_found ? "FOUND" : "NOT_FOUND");
        if (rtc_valid) printf("SENSOR RTC %02u:%02u\n", rtc_hour, rtc_minute);
        if (environment_valid) printf("SENSOR ENV temp_tenths=%d humidity_tenths=%u\n", temperature_tenths, humidity_tenths);
    }
    fflush(stdout);
}

/* Probe the recovered addresses and take one diagnostic reading from each. */
static void probe_sensors(void)
{
    rtc_valid = false; environment_valid = false; rtc_found = false; environment_found = false;
    i2c_config_t config = { .mode = I2C_MODE_MASTER, .sda_io_num = 4, .scl_io_num = 5,
        .sda_pullup_en = GPIO_PULLUP_ENABLE, .scl_pullup_en = GPIO_PULLUP_ENABLE,
        .master.clk_speed = 100000 };
    esp_err_t error = i2c_param_config(I2C_NUM_0, &config);
    if (error == ESP_OK) error = i2c_driver_install(I2C_NUM_0, I2C_MODE_MASTER, 0, 0, 0);
    if (error != ESP_OK) { printf("SENSOR I2C ERROR %d\n", (int)error); return; }
    const unsigned addresses[] = { 0x32, 0x38 };
    for (unsigned i = 0; i < sizeof(addresses) / sizeof(addresses[0]); ++i) {
        i2c_cmd_handle_t cmd = i2c_cmd_link_create();
        if (!cmd) continue;
        i2c_master_start(cmd);
        i2c_master_write_byte(cmd, (uint8_t)(addresses[i] << 1), true);
        i2c_master_stop(cmd);
        error = i2c_master_cmd_begin(I2C_NUM_0, cmd, pdMS_TO_TICKS(50));
        i2c_cmd_link_delete(cmd);
        if (addresses[i] == 0x32) rtc_found = error == ESP_OK;
        if (addresses[i] == 0x38) environment_found = error == ESP_OK;
        printf("SENSOR I2C %02x %s\n", addresses[i], error == ESP_OK ? "FOUND" : "NOT_FOUND");
    }
    if (rtc_found) {
        uint8_t reg = 0, raw[7];
        if (i2c_master_write_read_device(I2C_NUM_0, 0x32, &reg, 1, raw, sizeof(raw), pdMS_TO_TICKS(100)) == ESP_OK) {
            uint8_t control_reg=0xe0, control[2];
            xggf_clock checked={0};
            rtc_valid=i2c_master_write_read_device(I2C_NUM_0,0x32,&control_reg,1,control,2,pdMS_TO_TICKS(100))==ESP_OK && rtc_decode(raw,control,now_ms(),&checked);
            if(rtc_valid) {rtc_hour=checked.minute/60;rtc_minute=checked.minute%60;}
        }
    }
    if (environment_found) {
        const uint8_t init[] = {0xbe, 0x08, 0x00}, trigger[] = {0xac, 0x33, 0x00};
        uint8_t raw[6];
        if (i2c_master_write_to_device(I2C_NUM_0, 0x38, init, sizeof(init), pdMS_TO_TICKS(100)) == ESP_OK) {
            vTaskDelay(pdMS_TO_TICKS(20));
            if (i2c_master_write_to_device(I2C_NUM_0, 0x38, trigger, sizeof(trigger), pdMS_TO_TICKS(100)) == ESP_OK) {
                vTaskDelay(pdMS_TO_TICKS(100));
                if (i2c_master_read_from_device(I2C_NUM_0, 0x38, raw, sizeof(raw), pdMS_TO_TICKS(100)) == ESP_OK && !(raw[0] & 0x80)) {
                    uint32_t h = ((uint32_t)raw[1] << 12) | ((uint32_t)raw[2] << 4) | (raw[3] >> 4);
                    uint32_t t = ((uint32_t)(raw[3] & 15) << 16) | ((uint32_t)raw[4] << 8) | raw[5];
                    humidity_tenths = (unsigned)((uint64_t)h * 1000 / 1048576);
                    temperature_tenths = (int)((uint64_t)t * 2000 / 1048576) - 500;
                    environment_valid = (raw[0] & 0x08) != 0 && humidity_tenths <= 1000;
                }
            }
        }
    }
    sensor_probed = true;
    i2c_driver_delete(I2C_NUM_0);
}

static void toggle_microphone(status_model *model, uint32_t now)
{
    if (audio_active()) {
        puts("MIC BUSY"); wifi_link_report("MIC BUSY");
    } else if (model->audio_session) {
        microphone_stop(); status_audio_stop(model);
        puts("MIC DONE"); wifi_link_report("MIC DONE");
    } else {
        status_audio_start(model); microphone_probe(now);
    }
}

static void identify(status_model *model, uint32_t now)
{
    status_identify(model, now);
    if (!model->muted) buzzer_identify(now);
}

static void toggle_mute(status_model *model, uint32_t now)
{
    status_toggle_mute(model, now);
    puts(model->muted ? "MUTE ON" : "MUTE OFF");
    wifi_link_report(model->muted ? "MUTE ON" : "MUTE OFF");
}

static const char *button_ui_mode_name(button_ui_mode mode)
{
    static const char *const names[] = { "HOME", "MENU", "LINK" };
    return (unsigned)mode < sizeof(names) / sizeof(names[0]) ? names[mode] : "UNKNOWN";
}

static const char *button_menu_row_name(button_menu_row row)
{
    static const char *const names[] = { "MIC", "LINK", "BRIGHTNESS", "VOLUME", "IDENTIFY", "STOP FOCUS", "MUTE", "CLOSE" };
    return (unsigned)row < sizeof(names) / sizeof(names[0]) ? names[row] : "UNKNOWN";
}

static void report_button_transition(const button_transition *transition)
{
    char report[160];
    snprintf(report, sizeof(report), "BUTTON TRANSITION %s FROM %s/%s/%u/REC%u TO %s/%s/%u/REC%u",
        button_action_name(transition->action),
        button_ui_mode_name(transition->from.mode), button_menu_row_name(transition->from.row),
        transition->from.link_page, transition->from.recording,
        button_ui_mode_name(transition->to.mode), button_menu_row_name(transition->to.row),
        transition->to.link_page, transition->to.recording);
    puts(report);
    wifi_link_report(report);
}

static void report_menu(const status_model *model)
{
    static const char *rows[] = { "MIC", "LINK", "BRIGHTNESS", "VOLUME", "IDENTIFY", "STOP FOCUS", "MUTE", "CLOSE" };
    const button_actions_state *state = &model->button_actions;
    char report[40];
    if (state->mode == BUTTON_UI_HOME) snprintf(report, sizeof(report), "MENU CLOSED");
    else if (state->mode == BUTTON_UI_LINK) snprintf(report, sizeof(report), "MENU LINK %u", state->link_page);
    else snprintf(report, sizeof(report), "MENU ROW %s", rows[state->row]);
    puts(report); wifi_link_report(report);
}

static xggf_clock desktop_clock;
static void *pomodoro;
static uint32_t pomodoro_activity;

#ifdef XGGF_RTC_WRITE
/* Recovered stock writer at 0x420018BE: register zero followed by seven BCD bytes.
   Chip identity and backup retention still require physical validation. */
static bool rtc_write_clock(const xggf_clock *c)
{
    i2c_config_t config={.mode=I2C_MODE_MASTER,.sda_io_num=4,.scl_io_num=5,
        .sda_pullup_en=GPIO_PULLUP_ENABLE,.scl_pullup_en=GPIO_PULLUP_ENABLE,.master.clk_speed=100000};
    if(i2c_param_config(I2C_NUM_0,&config)!=ESP_OK || i2c_driver_install(I2C_NUM_0,I2C_MODE_MASTER,0,0,0)!=ESP_OK) return false;
    uint8_t reg=0xe0,controls[3]={0xe0,0,0},data[8]={0},readback[7];
    bool ok=i2c_master_write_read_device(I2C_NUM_0,0x32,&reg,1,&controls[1],2,pdMS_TO_TICKS(100))==ESP_OK;
    controls[1]|=0x20;
    controls[2]=(controls[2]|0x20)&~0x50;
    ok=ok && rtc_encode(c,&data[1]) &&
        i2c_master_write_to_device(I2C_NUM_0,0x32,controls,3,pdMS_TO_TICKS(100))==ESP_OK &&
        i2c_master_write_to_device(I2C_NUM_0,0x32,data,8,pdMS_TO_TICKS(100))==ESP_OK;
    reg=0;
    ok=ok && i2c_master_write_read_device(I2C_NUM_0,0x32,&reg,1,readback,7,pdMS_TO_TICKS(100))==ESP_OK && memcmp(&readback[1],&data[2],6)==0;
    i2c_driver_delete(I2C_NUM_0);
    puts(ok ? "RTC SYNC OK" : "RTC ERROR write or readback");
    return ok;
}
#endif

static void run_button_action(status_model *model, button_action action, uint32_t now)
{
    if (action != BUTTON_ACTION_NONE) pomodoro_activity = now;
    switch (action) {
        case BUTTON_ACTION_VOLUME_NEXT: settings_cycle_volume(); model->volume=settings_volume(); break;
        case BUTTON_ACTION_POMODORO_TOGGLE:
            status_audio_stop(model);
            vb_pomodoro_toggle(pomodoro, now); model->pomodoro_alarm = false; break;
        case BUTTON_ACTION_POMODORO_STOP:
            vb_pomodoro_stop(pomodoro); model->pomodoro_alarm = false; break;
        case BUTTON_ACTION_MIC_TOGGLE: toggle_microphone(model, now); break;
        case BUTTON_ACTION_RECORDING_STOP:
            if (audio_request_stop(now)) {
                const char *request = "{\"version\":1,\"event\":\"audio.stop.request\"}";
                puts(request);
                wifi_link_report(request);
            }
            break;
        case BUTTON_ACTION_IDENTIFY: identify(model, now); break;
        case BUTTON_ACTION_VIEW_NEXT: status_next_view(model); break;
        case BUTTON_ACTION_SOURCE_OPEN: {
            const char *request = "{\"version\":1,\"event\":\"button\",\"button\":\"K2\",\"action\":\"press\"}";
            puts(request);
            wifi_link_report(request);
            break;
        }
        case BUTTON_ACTION_MUTE_TOGGLE: toggle_mute(model, now); buzzer_mute(model->muted); break;
        case BUTTON_ACTION_BRIGHTNESS_NEXT:
            if (!settings_cycle_brightness()) puts("UI SAVE ERROR");
            model->brightness = panel_brightness();
            break;
        case BUTTON_ACTION_MENU_OPEN: status_home(model); break;
        case BUTTON_ACTION_NONE:
        case BUTTON_ACTION_MENU_NEXT_ROW:
        case BUTTON_ACTION_MENU_CLOSE:
        case BUTTON_ACTION_LINK_OPEN:
        case BUTTON_ACTION_LINK_NEXT_PAGE:
        case BUTTON_ACTION_LINK_BACK:
        case BUTTON_ACTION_IGNORED_LONG_PRESS:
        case BUTTON_ACTION_MENU_TIMEOUT:
        case BUTTON_ACTION_EXTERNAL_RESET:
        case BUTTON_ACTION_RECORDING_STARTED:
        case BUTTON_ACTION_RECORDING_FINISHED:
            break;
    }
}

static void handle_button_event(status_model *model, button_event event, uint32_t now)
{
    char report[48];
    snprintf(report, sizeof(report), "BUTTON %s", button_event_name(event));
    puts(report); wifi_link_report(report);
    button_transition result = button_actions_handle(&model->button_actions, event, now);
    report_button_transition(&result);
    run_button_action(model, result.action, now);
    if (result.from.mode != BUTTON_UI_HOME || result.to.mode != BUTTON_UI_HOME ||
        result.action == BUTTON_ACTION_MENU_OPEN) report_menu(model);
}

#if defined(XGGF_BUTTON_AUTOPILOT)
static void report_autopilot_transition(const button_transition *result)
{
    printf("BUTTON TEST ACTION %s FROM %s/%s/%u/REC%u TO %s/%s/%u/REC%u\n",
        button_action_name(result->action),
        button_ui_mode_name(result->from.mode), button_menu_row_name(result->from.row),
        result->from.link_page, result->from.recording,
        button_ui_mode_name(result->to.mode), button_menu_row_name(result->to.row),
        result->to.link_page, result->to.recording);
}

static bool button_autopilot_command(status_model *model, const cJSON *root, const char *event)
{
    if (!strcmp(event, "device.test.status")) {
        printf("BUTTON TEST STATUS PAGE %u PHASE %u RUN %u LEFT %u COUNT %u CLOCK %u %u:%u VOLUME %u MUTED %u\n",
            (unsigned)model->view,(unsigned)model->pomodoro.phase,(unsigned)model->pomodoro.run,
            (unsigned)model->pomodoro.remaining_ms,(unsigned)model->pomodoro.completed,
            model->rtc_valid,model->rtc_hour,model->rtc_minute,model->volume,model->muted);
        return true;
    }
    if (!strcmp(event, "device.test.buttons.reset")) {
        button_actions_init(&model->button_actions);
        puts("BUTTON TEST RESET HOME/MIC/0/REC0");
        return true;
    }
    if (!strcmp(event, "device.test.buttons.external_reset")) {
        button_transition result = button_actions_external_reset(&model->button_actions);
        report_autopilot_transition(&result);
        return true;
    }
    if (!strcmp(event, "device.test.buttons.recording")) {
        const cJSON *active = cJSON_GetObjectItemCaseSensitive(root, "active");
        if (!cJSON_IsBool(active)) { puts("BUTTON TEST ERROR recording requires boolean active"); return true; }
        button_transition result = button_actions_set_recording(&model->button_actions,
                                                               cJSON_IsTrue(active));
        report_autopilot_transition(&result);
        return true;
    }
    if (!strcmp(event, "device.test.buttons.tick")) {
        const cJSON *advance = cJSON_GetObjectItemCaseSensitive(root, "advance_ms");
        if (!cJSON_IsNumber(advance) || advance->valuedouble < 0 || advance->valuedouble > UINT32_MAX) {
            puts("BUTTON TEST ERROR tick requires bounded advance_ms");
            return true;
        }
        uint32_t now = model->button_actions.last_activity_ms + (uint32_t)advance->valuedouble;
        button_transition result = button_actions_tick(&model->button_actions, now);
        report_autopilot_transition(&result);
        return true;
    }
    if (!strcmp(event, "device.test.buttons.gesture")) {
        const cJSON *gesture = cJSON_GetObjectItemCaseSensitive(root, "gesture");
        if (!cJSON_IsString(gesture) || !gesture->valuestring) {
            puts("BUTTON TEST ERROR gesture requires string");
            return true;
        }
        for (int candidate = BUTTON_K0_SHORT; candidate <= BUTTON_K2_LONG; ++candidate) {
            button_event button = (button_event)candidate;
            if (strcmp(gesture->valuestring, button_event_name(button))) continue;
            button_transition result = button_actions_handle(&model->button_actions, button, now_ms());
            report_autopilot_transition(&result);
            if(cJSON_IsTrue(cJSON_GetObjectItemCaseSensitive(root,"effects"))) run_button_action(model,result.action,now_ms());
            return true;
        }
        puts("BUTTON TEST ERROR unknown gesture");
        return true;
    }
    return false;
}
#endif

static void receive_line(void *context, char *input, bool usb)
{
    status_model *model = context;
    const char *end = NULL;
    cJSON *root = cJSON_ParseWithOpts(input, &end, 1);
    if (!cJSON_IsObject(root)) { cJSON_Delete(root); return; }
    const cJSON *version = cJSON_GetObjectItemCaseSensitive(root, "version");
    const cJSON *event = cJSON_GetObjectItemCaseSensitive(root, "event");
    if (cJSON_IsNumber(version) && version->valuedouble == 1 && cJSON_IsString(event) && event->valuestring) {
#if defined(XGGF_BUTTON_AUTOPILOT)
        if (usb && button_autopilot_command(model, root, event->valuestring)) {
            cJSON_Delete(root);
            return;
        }
#endif
        status_transport_update(model, usb);
        bool hello = strcmp(event->valuestring, "device.hello") == 0;
        if (usb) wifi_link_usb_activity();
        if (wifi_link_pairing_event(input, event->valuestring, usb)) {
            cJSON_Delete(root);
            return;
        }
        bool handled = usb && wifi_link_usb_event(event->valuestring, root);
        if (!handled && strcmp(event->valuestring, "device.audio.start") == 0) {
            const cJSON *rate = cJSON_GetObjectItemCaseSensitive(root, "sample_rate");
            const cJSON *duration = cJSON_GetObjectItemCaseSensitive(root, "duration_ms");
            handled = true;
            if (!cJSON_IsNumber(rate) || !cJSON_IsNumber(duration) ||
                !microphone_set_rate((uint32_t)rate->valuedouble) ||
                !audio_start((uint32_t)rate->valuedouble, (uint32_t)duration->valuedouble, now_ms())) {
                puts("AUDIO ERROR start"); wifi_link_report("AUDIO ERROR start"); cJSON_Delete(root); return;
            }
            audio_armed_from_usb = usb;
            audio_hold_deadline = now_ms() + (uint32_t)duration->valuedouble + AUDIO_HOLD_MARGIN_MS;
            button_transition recording_started = status_set_recording(model, true);
            report_button_transition(&recording_started);
        }
        if (!handled && strcmp(event->valuestring, "device.audio.stop") == 0) {
            audio_stop(); handled = true;
        }
        if (!handled && strcmp(event->valuestring, "device.mic") == 0) {
            if (audio_active()) { puts("MIC BUSY"); wifi_link_report("MIC BUSY"); cJSON_Delete(root); return; }
            status_audio_start(model); microphone_probe(now_ms()); handled = true;
        }
        if (!handled) {
            button_actions_state before = model->button_actions;
            handled = status_event(model, event->valuestring, now_ms());
            if (handled && (before.mode != model->button_actions.mode ||
                            before.row != model->button_actions.row ||
                            before.link_page != model->button_actions.link_page)) {
                button_transition reset = {
                    .action = BUTTON_ACTION_EXTERNAL_RESET,
                    .from = before,
                    .to = model->button_actions,
                };
                report_button_transition(&reset);
            }
        }
        if (strcmp(event->valuestring, "device.heartbeat") == 0) {
            const cJSON *h=cJSON_GetObjectItemCaseSensitive(root,"hour");
            const cJSON *m=cJSON_GetObjectItemCaseSensitive(root,"minute");
            const cJSON *d=cJSON_GetObjectItemCaseSensitive(root,"day");
            if(cJSON_IsNumber(h) && cJSON_IsNumber(m) && cJSON_IsNumber(d) &&
               h->valuedouble==h->valueint && m->valuedouble==m->valueint && d->valuedouble==d->valueint &&
               clock_sync(&desktop_clock,(unsigned)d->valueint,(unsigned)h->valueint,(unsigned)m->valueint,now_ms())) {
#ifdef XGGF_RTC_WRITE
                static uint32_t last_rtc_day, last_rtc_minute;
                if(last_rtc_day!=desktop_clock.day || last_rtc_minute!=desktop_clock.minute) {
                    if(rtc_write_clock(&desktop_clock)) {last_rtc_day=desktop_clock.day; last_rtc_minute=desktop_clock.minute;}
                }
#endif
                model->rtc_valid=true; model->rtc_hour=h->valueint; model->rtc_minute=m->valueint; model->rtc_read_ms=now_ms();
                if(vb_pomodoro_set_day(pomodoro,desktop_clock.day)) settings_pomodoro_save(pomodoro);
            }
        }
        if (handled && strcmp(event->valuestring, "device.identify") == 0 && !model->muted) buzzer_identify(now_ms());
        if (handled && !hello && strcmp(event->valuestring, "device.heartbeat") != 0) {
            char report[96]; snprintf(report, sizeof(report), "STATE %.80s", event->valuestring);
            puts(report); wifi_link_report(report);
        }
        if (hello) announce();
    }
    cJSON_Delete(root);
}

void app_main(void)
{
    /* The daemon distinguishes foreign ESP-IDF applications by their log
     * format; diagnostics from this firmware use our own plain lines. */
    esp_log_level_set("*", ESP_LOG_NONE);
    usb_serial_jtag_driver_config_t usb = { .tx_buffer_size = 1024, .rx_buffer_size = 8192 };
    if (usb_serial_jtag_driver_install(&usb) != ESP_OK) return;
    usb_serial_jtag_vfs_use_driver();
    setvbuf(stdout, NULL, _IONBF, 0);
    uint8_t *transfer = heap_caps_malloc(PANEL_TRANSFER_BYTES, MALLOC_CAP_DMA | MALLOC_CAP_8BIT);
    if (!transfer) { puts("PANEL ERROR no DMA memory"); return; }
    spi_bus_config_t bus = { .mosi_io_num = PANEL_DATA_GPIO, .miso_io_num = -1, .sclk_io_num = -1,
        .quadwp_io_num = -1, .quadhd_io_num = -1, .max_transfer_sz = PANEL_TRANSFER_BYTES };
    esp_err_t error = spi_bus_initialize(SPI2_HOST, &bus, SPI_DMA_CH_AUTO);
    if (error != ESP_OK) { printf("PANEL ERROR bus %d\n", (int)error); free(transfer); return; }
    spi_device_interface_config_t config = { .mode = 0, .clock_speed_hz = PANEL_SPI_HZ, .spics_io_num = -1, .queue_size = 1 };
    spi_device_handle_t spi;
    error = spi_bus_add_device(SPI2_HOST, &config, &spi);
    if (error != ESP_OK) { printf("PANEL ERROR device %d\n", (int)error); spi_bus_free(SPI2_HOST); free(transfer); return; }
    status_model model; status_init(&model);
    pomodoro = vb_pomodoro_new();
    gpio_config_t button_config = {
        .pin_bit_mask = (1ULL << 8) | (1ULL << 9) | (1ULL << 2),
        .mode = GPIO_MODE_INPUT,
        .pull_up_en = GPIO_PULLUP_DISABLE,
        .pull_down_en = GPIO_PULLDOWN_DISABLE,
        .intr_type = GPIO_INTR_DISABLE,
    };
    buttons_state buttons;
    bool buttons_ready = gpio_config(&button_config) == ESP_OK;
    if (buttons_ready) {
        const gpio_num_t pins[3] = { GPIO_NUM_8, GPIO_NUM_9, GPIO_NUM_2 };
        bool levels[3];
        for (unsigned i = 0; i < 3; ++i) levels[i] = gpio_get_level(pins[i]) == 0;
        buttons_init(&buttons, levels, now_ms());
        puts("BUTTONS READY");
    } else puts("BUTTONS ERROR");
    if (!buzzer_init()) puts("BUZZER ERROR init");
    if (!microphone_init()) puts("MIC ERROR init");
    wifi_link_set_wait_drain(drain_microphone);
    announce();
    probe_sensors();
    status_sensors(&model, rtc_valid, rtc_hour, rtc_minute, environment_valid, temperature_tenths, humidity_tenths, now_ms());
    uint32_t sensors_at = now_ms();
    if (wifi_link_init()) announce(); else puts("WIFI UNAVAILABLE");
    settings_init();
    settings_pomodoro_load(pomodoro);
    model.volume=settings_volume();
    model.brightness = panel_brightness();
    uint32_t diagnostics_at = now_ms(), panel_us = 0;
    size_t used = 0; bool dropping = false;
    for (;;) {
        uint8_t input[256];
        int count = usb_serial_jtag_read_bytes(input, sizeof(input), 0);
        for (int i = 0; i < count; ++i) {
            uint8_t b = input[i];
            if (b == '\n') {
                if (!dropping && used) { line[used] = 0; receive_line(&model, line, true); }
                used = 0; dropping = false;
            } else if (!dropping) {
                if (b == 0 || used == MAX_LINE) { dropping = true; used = 0; }
                else line[used++] = (char)b;
            }
        }
        uint32_t now = now_ms();
        if (buttons_ready) {
            const gpio_num_t pins[3] = { GPIO_NUM_8, GPIO_NUM_9, GPIO_NUM_2 };
            bool levels[3], long_fired[3];
            for (unsigned i = 0; i < 3; ++i) levels[i] = gpio_get_level(pins[i]) == 0;
            button_event events[3];
            unsigned event_count = buttons_update(&buttons, levels, now, events);
            for (unsigned i = 0; i < 3; ++i) long_fired[i] = buttons.key[i].long_fired;
            status_buttons(&model, levels, long_fired);
            for (unsigned i = 0; i < event_count; ++i) handle_button_event(&model, events[i], now);
        }
        buzzer_tick(now);
        if (audio_active()) {
            if (audio_stop_expired(now)) {
                audio_stop();
                puts("AUDIO STOP TIMEOUT"); wifi_link_report("AUDIO STOP TIMEOUT");
            }
            int32_t raw[512];
            for (unsigned read = 0; read < 16; ++read) {
                size_t count = microphone_read(raw, 512);
                if (!count) break;
                audio_push(raw, count, now);
            }
            if (audio_armed_from_usb || now >= audio_hold_deadline || !wifi_link_audio_hold())
                audio_tick(now, audio_output, NULL);
            if (!audio_active()) {
                button_transition recording_finished = status_set_recording(&model, false);
                report_button_transition(&recording_finished);
                microphone_set_rate(8000);
            }
        } else {
            if (!model.audio_session) microphone_stop();
            microphone_tick(now);
            uint32_t audio_mean, audio_peak;
            if (microphone_take_level(&audio_mean, &audio_peak)) status_audio(&model, audio_mean, now);
            if (model.audio_session && !microphone_active()) status_audio_stop(&model);
        }
        wifi_link_tick(receive_line, &model);
        status_network(&model, wifi_link_configured(), wifi_link_online());
        link_status_info link_info = wifi_link_get_status();
        status_link_update(&model, &link_info);
        if ((uint32_t)(now - sensors_at) >= 60000) {
            probe_sensors();
            status_sensors(&model, rtc_valid, rtc_hour, rtc_minute, environment_valid, temperature_tenths, humidity_tenths, now_ms());
            sensors_at = now_ms();
            now = sensors_at;
        }
        button_transition button_timeout = button_actions_tick(&model.button_actions, now);
        if (button_timeout.action != BUTTON_ACTION_NONE) {
            report_button_transition(&button_timeout);
            report_menu(&model);
        }
        if (!pomodoro) pomodoro = vb_pomodoro_new();
        clock_tick(&desktop_clock,now);
        if(desktop_clock.valid) {
            model.rtc_valid=true; model.rtc_hour=desktop_clock.minute/60; model.rtc_minute=desktop_clock.minute%60; model.rtc_read_ms=desktop_clock.at_ms;
            if(vb_pomodoro_set_day(pomodoro,desktop_clock.day)) settings_pomodoro_save(pomodoro);
        }
        uint32_t ended = vb_pomodoro_tick(pomodoro, now);
        if (ended) { status_audio_stop(&model); button_actions_external_reset(&model.button_actions); pomodoro_activity=now; buzzer_chime(ended,now); settings_pomodoro_save(pomodoro); model.view = STATUS_VIEW_POMODORO; model.pomodoro_alarm = true; model.pomodoro_alarm_at_ms=now; }
        vb_pomodoro_view(pomodoro, now, &model.pomodoro);
        if (model.view == STATUS_VIEW_POMODORO &&
            ((model.pomodoro.run == 2 && (uint32_t)(now-pomodoro_activity)>=1800000) ||
             (model.pomodoro.phase == 1 && model.pomodoro.run == 0 && (uint32_t)(now-pomodoro_activity)>=300000))) status_home(&model);
        model.button_actions.pomodoro_page = model.view == STATUS_VIEW_POMODORO;
        model.button_actions.pomodoro_active = model.pomodoro.run != 0 || model.pomodoro.phase != 0;
        status_tick(&model, now);
        status_render(&model, now, frame);
        panel_encode(frame, transfer);
        spi_transaction_t tx = { .length = PANEL_TRANSFER_BYTES * 8, .tx_buffer = transfer };
        int64_t panel_started = esp_timer_get_time();
        esp_err_t transmit_error = spi_device_transmit(spi, &tx);
        panel_us = (uint32_t)(esp_timer_get_time() - panel_started);
        if (transmit_error != ESP_OK) { puts("PANEL ERROR transmit"); break; }
        if ((uint32_t)(now - diagnostics_at) >= 60000) {
            printf("DIAG uptime_ms=%u heap_free=%u heap_min=%u stack_free_min=%u panel_us=%u audio_overruns=%u\n",
                (unsigned)now, (unsigned)esp_get_free_heap_size(), (unsigned)esp_get_minimum_free_heap_size(),
                (unsigned)uxTaskGetStackHighWaterMark(NULL), (unsigned)panel_us, (unsigned)audio_overruns());
            diagnostics_at = now;
        }
        if (!audio_active()) vTaskDelay(pdMS_TO_TICKS(20));
    }
    spi_bus_remove_device(spi); spi_bus_free(SPI2_HOST); free(transfer);
}
