/* The C side of pairing: what pair_store.c keeps in NVS, driven through the real Rust bridge against an in-memory NVS
 * that can fail. Build the bridge first (cargo build --manifest-path firmware-xggf/link/Cargo.toml), then:
 * cc -std=c11 -Wall -Wextra -Werror -Ifirmware-xggf/main -Ifirmware-xggf/tests/stubs firmware-xggf/main/pair_store.c
 *    firmware-xggf/tests/pair_store_test.c firmware-xggf/link/target/debug/libvibebuddy_xggf_link.a -lpthread -ldl -lm */
#include <assert.h>
#include <stdio.h>
#include <string.h>
#include "pair_store.h"

extern Bridge *vb_link_new(const unsigned char *seed);
extern bool vb_link_pairing(Bridge *, const unsigned char *, size_t, bool, unsigned, bool (*)(void *, const unsigned char *, size_t), void *,
                            void (*)(void *, unsigned, unsigned, const unsigned char *, size_t), void *);
extern unsigned vb_link_paired_count(const Bridge *);

#define KEY_A "AgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgI"
#define KEY_B "AwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwM"

typedef struct { char key[16]; unsigned char value[2048]; size_t len; bool used; } blob;
static blob blobs[8];
static bool fail_writes;

static blob *find(const char *key)
{
    for (size_t i = 0; i < sizeof(blobs) / sizeof(blobs[0]); ++i)
        if (blobs[i].used && !strcmp(blobs[i].key, key)) return &blobs[i];
    return NULL;
}

esp_err_t nvs_get_blob(nvs_handle_t handle, const char *key, void *out, size_t *len)
{
    (void)handle;
    const blob *found = find(key);
    if (!found) return ESP_ERR_NVS_NOT_FOUND;
    if (out) {
        if (*len < found->len) return ESP_ERR_NVS_INVALID_LENGTH;
        memcpy(out, found->value, found->len);
    }
    *len = found->len;
    return ESP_OK;
}

esp_err_t nvs_set_blob(nvs_handle_t handle, const char *key, const void *value, size_t len)
{
    (void)handle;
    if (fail_writes) return 0x1105;
    blob *slot = find(key);
    for (size_t i = 0; !slot && i < sizeof(blobs) / sizeof(blobs[0]); ++i)
        if (!blobs[i].used) slot = &blobs[i];
    assert(slot && len <= sizeof(slot->value));
    snprintf(slot->key, sizeof(slot->key), "%s", key);
    if (len) memcpy(slot->value, value, len);
    slot->len = len; slot->used = true;
    return ESP_OK;
}

esp_err_t nvs_erase_key(nvs_handle_t handle, const char *key)
{
    (void)handle;
    blob *found = find(key);
    if (!found) return ESP_ERR_NVS_NOT_FOUND;
    found->used = false;
    return ESP_OK;
}

esp_err_t nvs_commit(nvs_handle_t handle) { (void)handle; return ESP_OK; }

static nvs_handle_t store = 1;
static char replies[1024];

static void collect(void *context, unsigned kind, unsigned conn, const unsigned char *bytes, size_t len)
{
    (void)context; (void)conn;
    assert(kind == 7);
    size_t used = strlen(replies);
    assert(used + len + 2 < sizeof(replies));
    memcpy(replies + used, bytes, len);
    replies[used + len] = '\n'; replies[used + len + 1] = 0;
}

static const char *command(Bridge *bridge, const char *event, const char *key, const char *name)
{
    char line[256];
    if (key && name) snprintf(line, sizeof(line), "{\"version\":1,\"event\":\"%s\",\"key\":\"%s\",\"name\":\"%s\"}", event, key, name);
    else if (key) snprintf(line, sizeof(line), "{\"version\":1,\"event\":\"%s\",\"key\":\"%s\"}", event, key);
    else snprintf(line, sizeof(line), "{\"version\":1,\"event\":\"%s\"}", event);
    replies[0] = 0;
    assert(vb_link_pairing(bridge, (const unsigned char *)line, strlen(line), true, 0, pair_store_save, &store, collect, NULL));
    return replies;
}

static Bridge *boot(bool *loaded)
{
    static const unsigned char seed[32] = { 1 };
    Bridge *bridge = vb_link_new(seed);
    assert(bridge);
    *loaded = pair_store_load(bridge, &store);
    return bridge;
}

static void reset(void)
{
    memset(blobs, 0, sizeof(blobs));
    fail_writes = false;
}

static void store_legacy(const char *const *keys, const char *const *names, size_t count)
{
    unsigned char key_bytes[16 * 44] = {0}, name_bytes[16 * 33] = {0};
    for (size_t i = 0; i < count; ++i) {
        memcpy(key_bytes + i * 44, keys[i], strlen(keys[i]));
        memcpy(name_bytes + i * 33, names[i], strlen(names[i]));
    }
    assert(nvs_set_blob(store, "pairs", key_bytes, count * 44) == ESP_OK);
    assert(nvs_set_blob(store, "names", name_bytes, count * 33) == ESP_OK);
}

static void test_a_new_box_starts_empty_and_writes_nothing(void)
{
    reset();
    bool loaded;
    Bridge *bridge = boot(&loaded);
    assert(loaded && vb_link_paired_count(bridge) == 0);
    assert(!find("paired") && !find("pairs"));
    size_t len = 0;
    assert(pair_store_probe(store, &len) == ESP_ERR_NVS_NOT_FOUND);
}

static void test_a_pairing_survives_a_restart(void)
{
    reset();
    bool loaded;
    Bridge *bridge = boot(&loaded);
    assert(!strcmp(command(bridge, "device.pair", KEY_A, "desk"), "PAIRED " KEY_A "\n"));
    size_t len = 0;
    assert(pair_store_probe(store, &len) == ESP_OK);
    bridge = boot(&loaded);
    assert(loaded && !strcmp(command(bridge, "device.pairs", NULL, NULL), "PAIRS 1\nPAIR " KEY_A " desk\n"));
}

static void test_an_unpairing_survives_a_restart(void)
{
    reset();
    bool loaded;
    Bridge *bridge = boot(&loaded);
    command(bridge, "device.pair", KEY_A, "desk");
    assert(!strcmp(command(bridge, "device.unpair", KEY_A, NULL), "UNPAIRED " KEY_A "\n"));
    bridge = boot(&loaded);
    assert(loaded && vb_link_paired_count(bridge) == 0);
}

static void test_the_earlier_format_moves_over_and_is_removed(void)
{
    reset();
    const char *keys[] = { KEY_A, KEY_B }, *names[] = { "desk", "laptop" };
    store_legacy(keys, names, 2);
    size_t len = 0;
    assert(pair_store_probe(store, &len) == ESP_OK);
    bool loaded;
    Bridge *bridge = boot(&loaded);
    assert(loaded && !strcmp(command(bridge, "device.pairs", NULL, NULL), "PAIRS 2\nPAIR " KEY_A " desk\nPAIR " KEY_B " laptop\n"));
    assert(find("paired") && !find("pairs") && !find("names"));
    bridge = boot(&loaded);
    assert(loaded && vb_link_paired_count(bridge) == 2);
}

/* The bug this replaced: a name that isn't UTF-8 was saved, and then every boot refused to bring Wi-Fi up. */
static void test_a_bad_stored_name_no_longer_stops_the_boot(void)
{
    reset();
    const char *keys[] = { KEY_A }, *names[] = { "bad\xff" };
    store_legacy(keys, names, 1);
    bool loaded;
    Bridge *bridge = boot(&loaded);
    assert(loaded);
    assert(!strcmp(command(bridge, "device.pairs", NULL, NULL), "PAIRS 1\nPAIR " KEY_A " \n"));
}

static void test_a_bad_name_over_the_cable_is_refused_before_anything_is_saved(void)
{
    reset();
    bool loaded;
    Bridge *bridge = boot(&loaded);
    assert(!strcmp(command(bridge, "device.pair", KEY_A, "bad\xff"), "ERROR invalid_json\n"));
    assert(!find("paired") && vb_link_paired_count(bridge) == 0);
    bridge = boot(&loaded);
    assert(loaded && vb_link_paired_count(bridge) == 0);
}

static void test_a_failed_move_keeps_the_earlier_format_and_tries_again(void)
{
    reset();
    const char *keys[] = { KEY_A }, *names[] = { "desk" };
    store_legacy(keys, names, 1);
    fail_writes = true;
    bool loaded;
    Bridge *bridge = boot(&loaded);
    assert(loaded && vb_link_paired_count(bridge) == 1);
    assert(find("pairs") && find("names") && !find("paired"));
    fail_writes = false;
    bridge = boot(&loaded);
    assert(loaded && vb_link_paired_count(bridge) == 1);
    assert(find("paired") && !find("pairs") && !find("names"));
}

/* An empty list is still stored, or the earlier format would bring removed computers back. */
static void test_a_removed_computer_cannot_come_back_from_the_earlier_format(void)
{
    reset();
    bool loaded;
    Bridge *bridge = boot(&loaded);
    command(bridge, "device.pair", KEY_A, "desk");
    command(bridge, "device.unpair", KEY_A, NULL);
    assert(find("paired"));
    const char *keys[] = { KEY_A }, *names[] = { "desk" };
    store_legacy(keys, names, 1);
    bridge = boot(&loaded);
    assert(loaded && vb_link_paired_count(bridge) == 0);
}

static void test_what_cannot_be_read_is_reported_not_overwritten(void)
{
    reset();
    const unsigned char corrupt[] = { 9, 9, 9 };
    assert(nvs_set_blob(store, "paired", corrupt, sizeof(corrupt)) == ESP_OK);
    bool loaded;
    Bridge *bridge = boot(&loaded);
    assert(!loaded && vb_link_paired_count(bridge) == 0);
    assert(find("paired")->len == sizeof(corrupt));

    reset();
    assert(nvs_set_blob(store, "pairs", "short", 5) == ESP_OK);
    bridge = boot(&loaded);
    assert(!loaded && vb_link_paired_count(bridge) == 0);
    assert(find("pairs")->len == 5);
}

static void test_a_failed_save_changes_neither_memory_nor_storage(void)
{
    reset();
    bool loaded;
    Bridge *bridge = boot(&loaded);
    command(bridge, "device.pair", KEY_A, "desk");
    fail_writes = true;
    assert(!strcmp(command(bridge, "device.pair", KEY_B, "new"), "LINK SAVE ERROR\n"));
    assert(!strcmp(command(bridge, "device.unpair", KEY_A, NULL), "LINK SAVE ERROR\n"));
    assert(vb_link_paired_count(bridge) == 1);
    fail_writes = false;
    bridge = boot(&loaded);
    assert(loaded && !strcmp(command(bridge, "device.pairs", NULL, NULL), "PAIRS 1\nPAIR " KEY_A " desk\n"));
}

int main(void)
{
    test_a_new_box_starts_empty_and_writes_nothing();
    test_a_pairing_survives_a_restart();
    test_an_unpairing_survives_a_restart();
    test_the_earlier_format_moves_over_and_is_removed();
    test_a_bad_stored_name_no_longer_stops_the_boot();
    test_a_bad_name_over_the_cable_is_refused_before_anything_is_saved();
    test_a_failed_move_keeps_the_earlier_format_and_tries_again();
    test_a_removed_computer_cannot_come_back_from_the_earlier_format();
    test_what_cannot_be_read_is_reported_not_overwritten();
    test_a_failed_save_changes_neither_memory_nor_storage();
    puts("pair store tests passed");
    return 0;
}
