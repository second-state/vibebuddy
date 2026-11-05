#include "pair_store.h"

enum {
    MAX_PAIRS = 16,
    /* Version and count, then for each computer a 32-byte key, a length and up to 32 bytes of name. */
    STORED_BYTES = 2 + MAX_PAIRS * (32 + 1 + 32),
    LEGACY_KEYS_BYTES = MAX_PAIRS * 44,
    LEGACY_NAMES_BYTES = MAX_PAIRS * 33,
};

typedef bool (*pair_save)(void *, const unsigned char *, size_t);
extern int vb_link_restore(Bridge *, const unsigned char *, size_t);
extern bool vb_link_restore_legacy(Bridge *, const unsigned char *, size_t, const unsigned char *, size_t);
extern bool vb_link_save(const Bridge *, pair_save, void *);

/* One value, so a save either happens or doesn't. It is kept even when the list is empty: its presence is what says
 * the earlier format has been dealt with, so a computer removed here can't come back from it. */
static const char STORED[] = "paired", LEGACY_KEYS[] = "pairs", LEGACY_NAMES[] = "names";

bool pair_store_save(void *context, const unsigned char *bytes, size_t len)
{
    nvs_handle_t store = *(nvs_handle_t *)context;
    return nvs_set_blob(store, STORED, bytes, len) == ESP_OK && nvs_commit(store) == ESP_OK;
}

esp_err_t pair_store_probe(nvs_handle_t store, size_t *len)
{
    esp_err_t error = nvs_get_blob(store, STORED, NULL, len);
    return error == ESP_ERR_NVS_NOT_FOUND ? nvs_get_blob(store, LEGACY_KEYS, NULL, len) : error;
}

/* The two arrays earlier builds kept. Only once the new value is saved do they go; a failed save tries again at the
 * next boot, with the list in memory all the same. */
static bool load_legacy(Bridge *bridge, nvs_handle_t *store)
{
    static unsigned char keys[LEGACY_KEYS_BYTES], names[LEGACY_NAMES_BYTES];
    size_t keys_len = sizeof(keys), names_len = sizeof(names);
    esp_err_t error = nvs_get_blob(*store, LEGACY_KEYS, keys, &keys_len);
    if (error == ESP_ERR_NVS_NOT_FOUND) return true;
    if (error != ESP_OK) return false;
    if (nvs_get_blob(*store, LEGACY_NAMES, names, &names_len) != ESP_OK) names_len = 0;
    if (!vb_link_restore_legacy(bridge, keys, keys_len, names, names_len)) return false;
    if (vb_link_save(bridge, pair_store_save, store)) {
        (void)nvs_erase_key(*store, LEGACY_KEYS);
        (void)nvs_erase_key(*store, LEGACY_NAMES);
        (void)nvs_commit(*store);
    }
    return true;
}

bool pair_store_load(Bridge *bridge, nvs_handle_t *store)
{
    static unsigned char bytes[STORED_BYTES];
    size_t len = sizeof(bytes);
    esp_err_t error = nvs_get_blob(*store, STORED, bytes, &len);
    if (error == ESP_ERR_NVS_NOT_FOUND) return load_legacy(bridge, store);
    if (error != ESP_OK) return false;
    int restored = vb_link_restore(bridge, bytes, len);
    if (restored < 0) return false;
    if (restored > 0) (void)vb_link_save(bridge, pair_store_save, store);
    return true;
}
