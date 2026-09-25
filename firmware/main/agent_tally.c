#include "agent_tally.h"

#include "esp_check.h"
#include "nvs.h"
#include "nvs_flash.h"

#define TALLY_NAMESPACE "pomodoro"
#define KEY_DAY "day"
#define KEY_COMPLETED "completed"
#define KEY_FOCUS_S "focus_s"

static const char *TAG = "agent_tally";

esp_err_t agent_tally_init(void) {
  esp_err_t result = nvs_flash_init();
  if (result == ESP_ERR_NVS_NO_FREE_PAGES ||
      result == ESP_ERR_NVS_NEW_VERSION_FOUND) {
    // If the partition format is stale, erase and start over: it only holds one day's counts, no great loss.
    ESP_RETURN_ON_ERROR(nvs_flash_erase(), TAG, "擦除 NVS 失败");
    result = nvs_flash_init();
  }
  return result;
}

static uint32_t read_u32(nvs_handle_t handle, const char *key) {
  uint32_t value = 0;
  if (nvs_get_u32(handle, key, &value) != ESP_OK) {
    return 0;
  }
  return value;
}

esp_err_t agent_tally_load(agent_pomodoro_tally_t *tally) {
  tally->day = 0;
  tally->completed = 0;
  tally->focus_s = 0;
  nvs_handle_t handle;
  esp_err_t result = nvs_open(TALLY_NAMESPACE, NVS_READONLY, &handle);
  if (result == ESP_ERR_NVS_NOT_FOUND) {
    return ESP_OK;
  }
  if (result != ESP_OK) {
    return result;
  }
  tally->day = read_u32(handle, KEY_DAY);
  tally->completed = (unsigned)read_u32(handle, KEY_COMPLETED);
  tally->focus_s = read_u32(handle, KEY_FOCUS_S);
  nvs_close(handle);
  return ESP_OK;
}

esp_err_t agent_tally_save(const agent_pomodoro_tally_t *tally) {
  nvs_handle_t handle;
  ESP_RETURN_ON_ERROR(nvs_open(TALLY_NAMESPACE, NVS_READWRITE, &handle), TAG,
                      "打开 NVS 失败");
  esp_err_t result = nvs_set_u32(handle, KEY_DAY, tally->day);
  if (result == ESP_OK) {
    result = nvs_set_u32(handle, KEY_COMPLETED, (uint32_t)tally->completed);
  }
  if (result == ESP_OK) {
    result = nvs_set_u32(handle, KEY_FOCUS_S, tally->focus_s);
  }
  if (result == ESP_OK) {
    result = nvs_commit(handle);
  }
  nvs_close(handle);
  return result;
}
