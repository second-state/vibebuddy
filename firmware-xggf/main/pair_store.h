#ifndef XGGF_PAIR_STORE_H
#define XGGF_PAIR_STORE_H
#include <stdbool.h>
#include <stddef.h>
#include "nvs.h"

/* Where the paired computers live in NVS. The list and its rules belong to the Rust bridge
 * (link/src/pairing.rs); this only moves its bytes to and from storage. */
typedef struct Bridge Bridge;

/* Fills the bridge at boot. False if what is stored can't be read as a list. */
bool pair_store_load(Bridge *bridge, nvs_handle_t *store);
/* The save callback for the bridge; context is the nvs_handle_t. */
bool pair_store_save(void *context, const unsigned char *bytes, size_t len);
/* Whether a list is stored, in this format or the earlier one. Something other than found or not found is an error. */
esp_err_t pair_store_probe(nvs_handle_t store, size_t *len);

#endif
