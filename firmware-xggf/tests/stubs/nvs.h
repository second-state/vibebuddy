#ifndef XGGF_TEST_NVS_H
#define XGGF_TEST_NVS_H
#include <stddef.h>
#include <stdint.h>

/* The part of ESP-IDF's NVS that pair_store.c uses; pair_store_test.c implements it in memory. */
typedef uint32_t nvs_handle_t;
typedef int esp_err_t;
#define ESP_OK 0
#define ESP_ERR_NVS_NOT_FOUND 0x1102
#define ESP_ERR_NVS_INVALID_LENGTH 0x110c

esp_err_t nvs_get_blob(nvs_handle_t handle, const char *key, void *out, size_t *len);
esp_err_t nvs_set_blob(nvs_handle_t handle, const char *key, const void *value, size_t len);
esp_err_t nvs_erase_key(nvs_handle_t handle, const char *key);
esp_err_t nvs_commit(nvs_handle_t handle);

#endif
