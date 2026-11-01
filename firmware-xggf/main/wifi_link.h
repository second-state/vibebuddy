#ifndef XGGF_WIFI_LINK_H
#define XGGF_WIFI_LINK_H
#include <stdbool.h>
#include "cJSON.h"

typedef void (*wifi_line_handler)(void *context, char *line, bool usb);
bool wifi_link_init(void);
void wifi_link_announce(void);
bool wifi_link_usb_event(const char *event, const cJSON *root);
void wifi_link_usb_activity(void);
void wifi_link_tick(wifi_line_handler handler, void *context);
void wifi_link_report(const char *line);
void wifi_link_diagnose(void);
/* Snapshots the link counters as one recording starts, so the next LINK DIAG attributes changes to it. */
void wifi_link_diag_mark(void);
#endif
