#include "settings.h"
#include "panel.h"
#include "nvs.h"

void settings_init(void)
{
    nvs_handle_t store;
    uint8_t stored = 0;
    bool valid = nvs_open("xggf_ui", NVS_READWRITE, &store) == ESP_OK;
    if (valid) {
        valid = nvs_get_u8(store, "brightness", &stored) == ESP_OK;
        nvs_close(store);
    }
    panel_set_brightness(valid ? panel_brightness_or_default(stored) : 8);
}

bool settings_cycle_brightness(void)
{
    unsigned level = panel_brightness_next(panel_brightness());
    nvs_handle_t store;
    if (nvs_open("xggf_ui", NVS_READWRITE, &store) != ESP_OK) return false;
    bool saved = nvs_set_u8(store, "brightness", (uint8_t)level) == ESP_OK && nvs_commit(store) == ESP_OK;
    nvs_close(store);
    if (saved) panel_set_brightness(level);
    return saved;
}
