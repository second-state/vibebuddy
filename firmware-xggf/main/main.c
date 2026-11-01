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
#include "status.h"
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

static void audio_output(void *context, const char *output)
{
    (void)context; puts(output); wifi_link_report(output);
    if (!strncmp(output, "AUDIO END ", 10)) wifi_link_diagnose();
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
            unsigned h = raw[2] & 0x3f, m = raw[1] & 0x7f;
            rtc_hour = (h >> 4) * 10 + (h & 15); rtc_minute = (m >> 4) * 10 + (m & 15);
            rtc_valid = (h & 15) < 10 && (m & 15) < 10 && rtc_hour < 24 && rtc_minute < 60;
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

static void button_action(status_model *model, button_event event, uint32_t now)
{
    char report[48];
    snprintf(report, sizeof(report), "BUTTON %s", button_event_name(event));
    puts(report); wifi_link_report(report);
    switch (event) {
        case BUTTON_K0_SHORT:
            if (audio_active()) {
                puts("MIC BUSY"); wifi_link_report("MIC BUSY");
            } else if (model->audio_session) {
                microphone_stop(); status_audio_stop(model);
                puts("MIC DONE"); wifi_link_report("MIC DONE");
            } else {
                status_audio_start(model); microphone_probe(now);
            }
            break;
        case BUTTON_K0_LONG:
            status_identify(model, now);
            if (!model->muted) buzzer_identify(now);
            break;
        case BUTTON_K1_SHORT: status_next_view(model); break;
        case BUTTON_K1_LONG: status_home(model); break;
        case BUTTON_K2_SHORT:
            puts("{\"version\":1,\"event\":\"button\",\"button\":\"K2\",\"action\":\"press\"}");
            wifi_link_report("{\"version\":1,\"event\":\"button\",\"button\":\"K2\",\"action\":\"press\"}");
            break;
        case BUTTON_K2_LONG:
            status_toggle_mute(model, now);
            puts(model->muted ? "MUTE ON" : "MUTE OFF");
            wifi_link_report(model->muted ? "MUTE ON" : "MUTE OFF");
            break;
    }
}

static void receive_line(void *context, char *input, bool usb)
{
    status_model *model = context;
    const char *end = NULL;
    cJSON *root = cJSON_ParseWithOpts(input, &end, 1);
    if (!cJSON_IsObject(root)) { cJSON_Delete(root); return; }
    const cJSON *version = cJSON_GetObjectItemCaseSensitive(root, "version");
    const cJSON *event = cJSON_GetObjectItemCaseSensitive(root, "event");
    if (cJSON_IsNumber(version) && version->valuedouble == 1 && cJSON_IsString(event) && event->valuestring) {
        bool hello = strcmp(event->valuestring, "device.hello") == 0;
        if (usb) wifi_link_usb_activity();
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
            status_set_recording(model, true);
        }
        if (!handled && strcmp(event->valuestring, "device.audio.stop") == 0) { audio_stop(); handled = true; }
        if (!handled && strcmp(event->valuestring, "device.mic") == 0) {
            if (audio_active()) { puts("MIC BUSY"); wifi_link_report("MIC BUSY"); cJSON_Delete(root); return; }
            status_audio_start(model); microphone_probe(now_ms()); handled = true;
        }
        if (!handled) handled = status_event(model, event->valuestring, now_ms());
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
    announce();
    probe_sensors();
    status_sensors(&model, rtc_valid, rtc_hour, rtc_minute, environment_valid, temperature_tenths, humidity_tenths, now_ms());
    uint32_t sensors_at = now_ms();
    if (wifi_link_init()) announce(); else puts("WIFI UNAVAILABLE");
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
            for (unsigned i = 0; i < event_count; ++i) button_action(&model, events[i], now);
        }
        buzzer_tick(now);
        if (audio_active()) {
            int32_t raw[512];
            for (unsigned read = 0; read < 16; ++read) {
                size_t count = microphone_read(raw, 512);
                if (!count) break;
                audio_push(raw, count, now);
            }
            audio_tick(now, audio_output, NULL);
            if (!audio_active()) { status_set_recording(&model, false); microphone_set_rate(8000); }
        } else {
            if (!model.audio_session) microphone_stop();
            microphone_tick(now);
            uint32_t audio_mean, audio_peak;
            if (microphone_take_level(&audio_mean, &audio_peak)) status_audio(&model, audio_mean, now);
            if (model.audio_session && !microphone_active()) status_audio_stop(&model);
        }
        wifi_link_tick(receive_line, &model);
        if ((uint32_t)(now - sensors_at) >= 60000) {
            probe_sensors();
            status_sensors(&model, rtc_valid, rtc_hour, rtc_minute, environment_valid, temperature_tenths, humidity_tenths, now_ms());
            sensors_at = now_ms();
            now = sensors_at;
        }
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
