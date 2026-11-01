#ifndef XGGF_WIFI_LINK_H
#define XGGF_WIFI_LINK_H
#include <stdbool.h>
#include "cJSON.h"

typedef void (*wifi_line_handler)(void *context, char *line, bool usb);
bool wifi_link_init(void);
void wifi_link_announce(void);
/* Indicator facts for the connection bar: configured means Wi-Fi credentials are stored,
   online means the station currently holds an IP address. */
bool wifi_link_configured(void);
bool wifi_link_online(void);
bool wifi_link_usb_event(const char *event, const cJSON *root);
void wifi_link_usb_activity(void);
void wifi_link_tick(wifi_line_handler handler, void *context);
void wifi_link_report(const char *line);
void wifi_link_diagnose(void);
/* Snapshots the link counters as one recording starts, so the next LINK DIAG attributes changes to it. */
void wifi_link_diag_mark(void);
/* Whether network-armed recording emission should wait for the peer to return. */
bool wifi_link_audio_hold(void);
/* Sets the callback drained while a send waits for a writable socket (keeps acquisition fed). */
void wifi_link_set_wait_drain(void (*drain)(void));
/* Best-effort encrypted delivery; false means the line did not go out and may be retried. */
bool wifi_link_deliver(const char *line);
#endif
