#include "settings.h"
#include "panel.h"
#include "nvs.h"
#include "volume.h"

static void load_volume(void);
void settings_init(void)
{
    nvs_handle_t store;
    uint8_t stored = 0;
    bool valid = nvs_open("xggf_ui", NVS_READWRITE, &store) == ESP_OK;
    if (valid) {
        valid = nvs_get_u8(store, "brightness", &stored) == ESP_OK;
        nvs_close(store);
    }
    load_volume();
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

#include "pomodoro.h"
void settings_pomodoro_load(void *p)
{
    nvs_handle_t store;
    uint32_t day=0,count=0,secs=0;
    if(nvs_open("xggf_ui",NVS_READONLY,&store)!=ESP_OK) return;
    if(nvs_get_u32(store,"pomo_day",&day)==ESP_OK &&
       nvs_get_u32(store,"pomo_count",&count)==ESP_OK &&
       nvs_get_u32(store,"pomo_secs",&secs)==ESP_OK)
        vb_pomodoro_restore(p,day,count,secs);
    nvs_close(store);
}
void settings_pomodoro_save(void *p)
{
    nvs_handle_t store;
    uint32_t day,count,secs;
    vb_pomodoro_tally(p,&day,&count,&secs);
    if(nvs_open("xggf_ui",NVS_READWRITE,&store)!=ESP_OK) return;
    if(nvs_set_u32(store,"pomo_day",day)==ESP_OK &&
       nvs_set_u32(store,"pomo_count",count)==ESP_OK &&
       nvs_set_u32(store,"pomo_secs",secs)==ESP_OK) nvs_commit(store);
    nvs_close(store);
}

#include "buzzer.h"
static unsigned volume=50;
unsigned settings_volume(void) { return volume; }
bool settings_cycle_volume(void)
{
    unsigned next=volume_next(volume);
    nvs_handle_t store;
    if(nvs_open("xggf_ui",NVS_READWRITE,&store)!=ESP_OK) return false;
    bool ok=nvs_set_u8(store,"volume",next)==ESP_OK && nvs_commit(store)==ESP_OK;
    nvs_close(store);
    if(ok) {volume=next; buzzer_volume(volume);}
    return ok;
}

static void load_volume(void) {
    nvs_handle_t store; uint8_t level=50;
    if(nvs_open("xggf_ui",NVS_READONLY,&store)==ESP_OK) { nvs_get_u8(store,"volume",&level); nvs_close(store); }
    if(volume_valid(level)) volume=level;
    buzzer_volume(volume);
}
