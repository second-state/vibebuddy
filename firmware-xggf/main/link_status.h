#ifndef XGGF_LINK_STATUS_H
#define XGGF_LINK_STATUS_H

#include <stdbool.h>

typedef enum {
    LINK_WIFI_NOT_SET,
    LINK_WIFI_CONNECTING,
    LINK_WIFI_JOINED,
    LINK_WIFI_FAILED,
    LINK_WIFI_DISCONNECTED
} link_wifi_state;

typedef struct {
    char ssid[33];
    char ip[16];
    link_wifi_state wifi_state;
    unsigned paired_count;
    bool lan_active;
} link_status_info;

#endif
