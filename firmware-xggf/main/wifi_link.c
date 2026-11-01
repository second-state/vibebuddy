#include "wifi_link.h"
#include <errno.h>
#include <ctype.h>
#include <fcntl.h>
#include <sys/poll.h>
#include <stdio.h>
#include <string.h>
#include <sys/socket.h>
#include <unistd.h>
#include "esp_event.h"
#include "bootloader_random.h"
#include "esp_netif.h"
#include "esp_random.h"
#include "esp_timer.h"
#include "esp_wifi.h"
#include "lwip/inet.h"
#include "nvs.h"
#include "nvs_flash.h"
#include "mdns.h"

typedef struct Bridge Bridge;
typedef void (*bridge_callback)(void *, unsigned, unsigned, const unsigned char *, size_t);
extern Bridge *vb_link_new(const unsigned char *seed);
extern bool vb_link_pair(Bridge *, const unsigned char *, size_t, const unsigned char *, size_t);
extern bool vb_link_valid_key(const unsigned char *, size_t);
extern bool vb_link_unpair(Bridge *, const unsigned char *, size_t, unsigned, bridge_callback, void *);
extern long vb_link_public(const Bridge *, unsigned char *, size_t);
extern void vb_lan_open(Bridge *, unsigned, const unsigned char *, unsigned, bridge_callback, void *);
extern void vb_lan_receive(Bridge *, unsigned, const unsigned char *, size_t, unsigned, bridge_callback, void *);
extern void vb_lan_closed(Bridge *, unsigned, unsigned, bridge_callback, void *);
extern void vb_lan_usb(Bridge *, unsigned, bridge_callback, void *);
extern void vb_lan_poll(Bridge *, unsigned, bridge_callback, void *);
extern long vb_lan_send(Bridge *, const unsigned char *, size_t, unsigned char *, size_t);

/* A closed TCP window on Wi-Fi only reopens after about one link RTT, which measured near 150 ms on
 * the target network, and mid-recording window stalls just over two seconds have been observed;
 * waiting less turns routine backpressure into a link teardown that loses whatever was in flight.
 * The wait drains the microphone in slices (WAIT_SLICE_MS) so patience cannot starve acquisition,
 * whose DMA hold is far smaller than this bound; the audio ring covers the stalled emission. */
enum { MAX_PAIRS = 16, KEY_LENGTH = 43, PORT = 7340, BUFFER = 8192, SEND_TIMEOUT_MS = 4000, WAIT_SLICE_MS = 40 };
enum { CLOSE_NONE, CLOSE_PROTOCOL, CLOSE_INPUT_OVERFLOW, CLOSE_PEER_EOF, CLOSE_RECEIVE, CLOSE_SEND, CLOSE_ENCRYPT, CLOSE_WIFI };
enum { SEND_NONE, SEND_PAYLOAD, SEND_NEWLINE };
static Bridge *bridge;
static nvs_handle_t store;
static char public_key[44], keys[MAX_PAIRS][44], ssid[33], password[65];
static char names[MAX_PAIRS][33];
static unsigned pair_count;
static int listener = -1, client = -1;
static bool peer, close_pending;
static unsigned close_reason;
static char pending_line[4609];
static bool pending;
static esp_netif_t *station;
static uint32_t last_connect_attempt;
static uint32_t last_ip;
static bool wifi_connected, connecting_report, connected_report, disconnected_report, disconnect_was_connected;
static bool scan_pending, scan_report;
static unsigned disconnect_reason;
static uint32_t last_wifi_down_ms;
static struct {
    unsigned accepts, peer_up, peer_down, encryption_failures, send_failures, partial_writes, send_blocked, send_timeouts;
    unsigned disconnects, wifi_disconnects, last_close, last_send_part, last_send_len, last_wifi_reason;
    unsigned last_send_ms, max_send_ms;
    int last_send_sent, last_send_errno, last_recv_errno;
} diagnostics;
/* Counters snapshot taken when a recording starts, so LINK DIAG can attribute changes to one recording
 * instead of the cumulative totals, which mixed sessions together during the errno-113 investigation. */
static struct {
    bool active;
    uint32_t started;
    int rssi;
    unsigned accepts, peer_up, peer_down, encryption_failures, send_failures, partial_writes, send_blocked, send_timeouts, disconnects, wifi_disconnects;
} mark;

static uint32_t milliseconds(void) { return (uint32_t)(esp_timer_get_time() / 1000); }
static int current_rssi(void)
{
    wifi_ap_record_t ap;
    return esp_wifi_sta_get_ap_info(&ap) == ESP_OK ? ap.rssi : 0;
}
static bool store_blob(const char *key, const void *data, size_t len)
{ return nvs_set_blob(store, key, data, len) == ESP_OK && nvs_commit(store) == ESP_OK; }
static bool store_pairs(void)
{
    if (pair_count) return nvs_set_blob(store, "pairs", keys, pair_count * sizeof(keys[0])) == ESP_OK &&
        nvs_set_blob(store, "names", names, pair_count * sizeof(names[0])) == ESP_OK && nvs_commit(store) == ESP_OK;
    esp_err_t a = nvs_erase_key(store, "pairs"), b = nvs_erase_key(store, "names");
    return (a == ESP_OK || a == ESP_ERR_NVS_NOT_FOUND) && (b == ESP_OK || b == ESP_ERR_NVS_NOT_FOUND) && nvs_commit(store) == ESP_OK;
}
static void set_peer(bool value)
{
    if (peer == value) return;
    peer = value;
    if (peer) ++diagnostics.peer_up; else ++diagnostics.peer_down;
}
static void request_close(unsigned reason)
{
    if (!close_pending) close_reason = reason;
    close_pending = true;
}
static void close_client(void)
{
    if (client >= 0) {
        close(client);
        ++diagnostics.disconnects;
        diagnostics.last_close = close_reason;
    }
    client = -1;
    set_peer(false);
}
/* Drained while a send waits for a writable socket, so a closed TCP window cannot starve the
 * microphone's small DMA queue. Set by the main loop before any recording can start. */
static void (*wait_drain)(void);

void wifi_link_set_wait_drain(void (*drain)(void))
{
    wait_drain = drain;
}

static bool send_part(const unsigned char *bytes, size_t len, unsigned part)
{
    size_t offset = 0;
    uint32_t started = milliseconds();
    int sent = 0, send_errno = 0;
    for (;;) {
        sent = send(client, bytes + offset, len - offset, MSG_DONTWAIT);
        if (sent > 0) offset += sent;
        if (sent >= 0 && offset == len) {
            uint32_t waited = milliseconds() - started;
            diagnostics.last_send_ms = waited;
            if (waited > diagnostics.max_send_ms) diagnostics.max_send_ms = waited;
            return true;
        }
        send_errno = sent < 0 ? errno : 0;
        diagnostics.last_send_part = part;
        diagnostics.last_send_len = len;
        diagnostics.last_send_sent = sent;
        diagnostics.last_send_errno = send_errno;
        if (sent >= 0) ++diagnostics.partial_writes;
        else if (send_errno == EAGAIN || send_errno == EWOULDBLOCK) ++diagnostics.send_blocked;
        else break;
        bool writable = false;
        for (uint32_t elapsed = milliseconds() - started; elapsed < SEND_TIMEOUT_MS && !writable;) {
            uint32_t slice = SEND_TIMEOUT_MS - elapsed;
            if (slice > WAIT_SLICE_MS) slice = WAIT_SLICE_MS;
            struct pollfd descriptor = { .fd = client, .events = POLLOUT };
            int ready = poll(&descriptor, 1, (int)slice);
            if (ready > 0) writable = true;
            else if (ready < 0 && errno != EINTR) { sent = -1; send_errno = errno; break; }
            else if (wait_drain) wait_drain();
            elapsed = milliseconds() - started;
        }
        if (sent < 0 && send_errno && send_errno != EAGAIN && send_errno != EWOULDBLOCK) break;
        if (writable) continue;
        ++diagnostics.send_timeouts;
        break;
    }
    uint32_t waited = milliseconds() - started;
    diagnostics.last_send_ms = waited;
    if (waited > diagnostics.max_send_ms) diagnostics.max_send_ms = waited;
    ++diagnostics.send_failures;
    diagnostics.last_send_part = part;
    diagnostics.last_send_len = len;
    diagnostics.last_send_sent = sent;
    diagnostics.last_send_errno = send_errno;
    request_close(CLOSE_SEND);
    return false;
}
static bool send_line(const unsigned char *line, size_t len)
{
    if (client < 0 || len > BUFFER) {
        ++diagnostics.send_failures;
        diagnostics.last_send_part = SEND_PAYLOAD;
        diagnostics.last_send_len = len;
        diagnostics.last_send_sent = -1;
        diagnostics.last_send_errno = 0;
        request_close(CLOSE_SEND);
        return false;
    }
    return send_part(line, len, SEND_PAYLOAD) && send_part((const unsigned char *)"\n", 1, SEND_NEWLINE);
}
static void bridge_output(void *context, unsigned kind, unsigned conn, const unsigned char *bytes, size_t len)
{
    (void)context; (void)conn;
    switch (kind) {
        case 1: case 2: send_line(bytes, len); break;
        case 3: request_close(CLOSE_PROTOCOL); break;
        case 4:
            if (pending || len >= sizeof(pending_line)) { request_close(CLOSE_INPUT_OVERFLOW); break; }
            memcpy(pending_line, bytes, len); pending_line[len] = 0; pending = true; break;
        case 5: set_peer(true); break;
        case 6: set_peer(false); break;
    }
}

/* Best-effort line delivery with a verdict, so audio can keep a failed line queued for retry. */
bool wifi_link_deliver(const char *line)
{
    if (!peer || client < 0) return false;
    unsigned char encoded[2048];
    long n = vb_lan_send(bridge, (const unsigned char *)line, strlen(line), encoded, sizeof(encoded));
    if (n < 0) { ++diagnostics.encryption_failures; request_close(CLOSE_ENCRYPT); return false; }
    return send_line(encoded, (size_t)n);
}

void wifi_link_report(const char *line)
{
    (void)wifi_link_deliver(line);
}

/* Whether a recording armed over the network should wait for its peer instead of emitting frames
 * nowhere: Wi-Fi is associated and configured, but no encrypted peer is connected right now. */
bool wifi_link_audio_hold(void)
{
    return bridge && ssid[0] && wifi_connected && !peer;
}

static const char *close_name(unsigned reason)
{
    static const char *names[] = { "none", "protocol", "input_overflow", "peer_eof", "receive", "send", "encryption", "wifi" };
    return reason < sizeof(names) / sizeof(names[0]) ? names[reason] : "unknown";
}

static const char *wifi_failure_reason(unsigned reason)
{
    switch (reason) {
        case WIFI_REASON_UNSPECIFIED: return "Unspecified";
        case WIFI_REASON_AUTH_EXPIRE: return "AuthenticationExpired";
        case WIFI_REASON_AUTH_LEAVE: return "AuthenticationLeft";
        case WIFI_REASON_ASSOC_EXPIRE: return "AssociationExpired";
        case WIFI_REASON_ASSOC_TOOMANY: return "AssociationTooMany";
        case WIFI_REASON_ASSOC_LEAVE: return "AssociationLeft";
        case WIFI_REASON_AUTH_FAIL: return "AuthenticationFailed";
        case WIFI_REASON_ASSOC_FAIL: return "AssociationFailed";
        case WIFI_REASON_CONNECTION_FAIL: return "ConnectionFailed";
        case WIFI_REASON_NO_AP_FOUND: return "NoAccessPointFound";
        case WIFI_REASON_BEACON_TIMEOUT: return "BeaconTimeout";
        case WIFI_REASON_HANDSHAKE_TIMEOUT: return "HandshakeTimeout";
        case WIFI_REASON_4WAY_HANDSHAKE_TIMEOUT: return "FourWayHandshakeTimeout";
        case WIFI_REASON_MIC_FAILURE: return "MicFailure";
        default: return NULL;
    }
}

void wifi_link_diag_mark(void)
{
    mark.active = true;
    mark.started = milliseconds();
    mark.rssi = current_rssi();
    mark.accepts = diagnostics.accepts;
    mark.peer_up = diagnostics.peer_up;
    mark.peer_down = diagnostics.peer_down;
    mark.encryption_failures = diagnostics.encryption_failures;
    mark.send_failures = diagnostics.send_failures;
    mark.partial_writes = diagnostics.partial_writes;
    mark.send_blocked = diagnostics.send_blocked;
    mark.send_timeouts = diagnostics.send_timeouts;
    mark.disconnects = diagnostics.disconnects;
    mark.wifi_disconnects = diagnostics.wifi_disconnects;
}

void wifi_link_diagnose(void)
{
    char text[640];
    uint32_t now = milliseconds();
    uint32_t wifi_down_ago = last_wifi_down_ms ? now - last_wifi_down_ms : 0;
    const char *reason_name = wifi_failure_reason(diagnostics.last_wifi_reason);
    int length = snprintf(text, sizeof(text),
        "LINK DIAG peer=%u tcp=%u accepts=%u peer_up=%u peer_down=%u encrypt_fail=%u send_fail=%u partial=%u blocked=%u timeouts=%u "
        "disconnects=%u wifi_disconnects=%u last_close=%s send_part=%u send_len=%u send_sent=%d send_errno=%d send_ms=%u max_send_ms=%u recv_errno=%d "
        "wifi_reason=%u wifi_reason_name=%s rssi=%d wifi_down_ago=%u",
        peer, client >= 0, diagnostics.accepts, diagnostics.peer_up, diagnostics.peer_down, diagnostics.encryption_failures,
        diagnostics.send_failures, diagnostics.partial_writes, diagnostics.send_blocked, diagnostics.send_timeouts, diagnostics.disconnects,
        diagnostics.wifi_disconnects, close_name(diagnostics.last_close), diagnostics.last_send_part, diagnostics.last_send_len,
        diagnostics.last_send_sent, diagnostics.last_send_errno, diagnostics.last_send_ms, diagnostics.max_send_ms,
        diagnostics.last_recv_errno, diagnostics.last_wifi_reason, reason_name ? reason_name : "unknown", current_rssi(), (unsigned)wifi_down_ago);
    if (mark.active && length > 0 && (size_t)length < sizeof(text)) {
        length += snprintf(text + length, sizeof(text) - length,
            " rec_ms=%u rec_rssi=%d rec_accepts=%u rec_peer_up=%u rec_peer_down=%u rec_encrypt_fail=%u rec_send_fail=%u rec_partial=%u "
            "rec_blocked=%u rec_timeouts=%u rec_disconnects=%u rec_wifi_disconnects=%u",
            (unsigned)(now - mark.started), mark.rssi, diagnostics.accepts - mark.accepts, diagnostics.peer_up - mark.peer_up,
            diagnostics.peer_down - mark.peer_down, diagnostics.encryption_failures - mark.encryption_failures,
            diagnostics.send_failures - mark.send_failures, diagnostics.partial_writes - mark.partial_writes,
            diagnostics.send_blocked - mark.send_blocked, diagnostics.send_timeouts - mark.send_timeouts,
            diagnostics.disconnects - mark.disconnects, diagnostics.wifi_disconnects - mark.wifi_disconnects);
    }
    puts(text);
    wifi_link_report(text);
}

void wifi_link_announce(void)
{
    if (!bridge) return;
    char text[160];
    puts("LINK NOISE"); wifi_link_report("LINK NOISE");
    snprintf(text, sizeof(text), "BOX KEY %s", public_key); puts(text); wifi_link_report(text);
    if (ssid[0]) { snprintf(text, sizeof(text), "WIFI NETWORK %s", ssid); puts(text); wifi_link_report(text); }
    esp_netif_ip_info_t ip;
    if (station && esp_netif_get_ip_info(station, &ip) == ESP_OK && ip.ip.addr != 0) {
        snprintf(text, sizeof(text), "WIFI ADDRESS " IPSTR, IP2STR(&ip.ip)); puts(text); wifi_link_report(text);
    }
    wifi_link_diagnose();
}

static void report_scan(void)
{
    enum { MAX_RESULTS = 12 };
    static wifi_ap_record_t records[MAX_RESULTS];
    uint16_t available = 0;
    esp_err_t error = esp_wifi_scan_get_ap_num(&available);
    if (error != ESP_OK) { printf("WIFI SCAN FAILED %d\n", (int)error); return; }
    uint16_t count = available < MAX_RESULTS ? available : MAX_RESULTS;
    if (count && esp_wifi_scan_get_ap_records(&count, records) != ESP_OK) { puts("WIFI SCAN FAILED records"); return; }
    for (uint16_t i = 0; i < count; ++i) {
        for (uint16_t j = i + 1; j < count; ++j) {
            if (records[j].rssi > records[i].rssi) {
                wifi_ap_record_t swap = records[i]; records[i] = records[j]; records[j] = swap;
            }
        }
    }
    printf("WIFI SEEN %u\n", (unsigned)count);
    for (uint16_t i = 0; i < count; ++i) {
        char name[33]; memcpy(name, records[i].ssid, 32); name[32] = 0;
        for (size_t j = 0; name[j]; ++j) if ((unsigned char)name[j] < 32) name[j] = '?';
        printf("WIFI SEEN %d %u %s\n", (int)records[i].rssi, (unsigned)records[i].primary, name);
    }
}

static void wifi_event(void *arg, esp_event_base_t base, int32_t id, void *data)
{
    (void)arg;
    if (base != WIFI_EVENT) return;
    if (id == WIFI_EVENT_STA_START) {
        connecting_report = ssid[0]; esp_wifi_connect(); last_connect_attempt = milliseconds();
    } else if (id == WIFI_EVENT_STA_CONNECTED) {
        wifi_connected = true; connected_report = true;
    } else if (id == WIFI_EVENT_STA_DISCONNECTED) {
        const wifi_event_sta_disconnected_t *event = data;
        disconnect_was_connected = wifi_connected; wifi_connected = false;
        disconnect_reason = event ? event->reason : 0; disconnected_report = true;
        last_wifi_down_ms = milliseconds();
        ++diagnostics.wifi_disconnects; diagnostics.last_wifi_reason = disconnect_reason;
    } else if (id == WIFI_EVENT_SCAN_DONE) {
        scan_pending = false; scan_report = true;
    }
}

bool wifi_link_init(void)
{
    if (nvs_flash_init() != ESP_OK || nvs_open("vibebuddy", NVS_READWRITE, &store) != ESP_OK) return false;
    unsigned char seed[32]; size_t len = sizeof(seed);
    esp_err_t seed_error = nvs_get_blob(store, "seed2", seed, &len);
    if (seed_error == ESP_ERR_NVS_NOT_FOUND) {
        /* The first experimental build used the RNG before enabling its
         * entropy source. Never silently change an identity that has pairs. */
        size_t pair_len = 0;
        esp_err_t pairs = nvs_get_blob(store, "pairs", NULL, &pair_len);
        if (pairs == ESP_OK && pair_len) { puts("IDENTITY MIGRATION REQUIRED"); return false; }
        if (pairs != ESP_OK && pairs != ESP_ERR_NVS_NOT_FOUND) return false;
        bootloader_random_enable();
        esp_fill_random(seed, sizeof(seed));
        bootloader_random_disable();
        if (!store_blob("seed2", seed, sizeof(seed))) return false;
    } else if (seed_error != ESP_OK || len != sizeof(seed)) return false;
    bridge = vb_link_new(seed); memset(seed, 0, sizeof(seed));
    if (!bridge || vb_link_public(bridge, (unsigned char *)public_key, sizeof(public_key) - 1) != KEY_LENGTH) return false;
    len = sizeof(keys);
    esp_err_t pairs_error = nvs_get_blob(store, "pairs", keys, &len);
    if (pairs_error == ESP_OK && len % sizeof(keys[0]) == 0 && len <= sizeof(keys)) {
        pair_count = len / sizeof(keys[0]);
        size_t names_len = sizeof(names);
        esp_err_t names_error = nvs_get_blob(store, "names", names, &names_len);
        if (names_error != ESP_ERR_NVS_NOT_FOUND && (names_error != ESP_OK || names_len != pair_count * sizeof(names[0]))) return false;
        for (unsigned i = 0; i < pair_count; ++i) {
            keys[i][43] = 0; names[i][32] = 0;
            if (!vb_link_pair(bridge, (const unsigned char *)keys[i], strlen(keys[i]), (const unsigned char *)names[i], strlen(names[i]))) return false;
        }
    } else if (pairs_error != ESP_ERR_NVS_NOT_FOUND) return false;
    len = sizeof(ssid); if (nvs_get_str(store, "ssid", ssid, &len) != ESP_OK) ssid[0] = 0;
    len = sizeof(password); if (nvs_get_str(store, "password", password, &len) != ESP_OK) password[0] = 0;
    if (esp_netif_init() != ESP_OK || esp_event_loop_create_default() != ESP_OK) return false;
    station = esp_netif_create_default_wifi_sta(); if (!station) return false;
    wifi_init_config_t config = WIFI_INIT_CONFIG_DEFAULT();
    if (esp_wifi_init(&config) != ESP_OK || esp_wifi_set_storage(WIFI_STORAGE_RAM) != ESP_OK ||
        esp_event_handler_register(WIFI_EVENT, ESP_EVENT_ANY_ID, wifi_event, NULL) != ESP_OK ||
        esp_wifi_set_mode(WIFI_MODE_STA) != ESP_OK) return false;
    if (ssid[0]) {
        wifi_config_t wifi = {0}; memcpy(wifi.sta.ssid, ssid, strlen(ssid)); memcpy(wifi.sta.password, password, strlen(password));
        if (esp_wifi_set_config(WIFI_IF_STA, &wifi) != ESP_OK) return false;
    }
    if (esp_wifi_start() != ESP_OK) return false;
    char hostname[24] = "vibebuddy-", instance[24] = "VibeBuddy ";
    for (unsigned i = 0; i < 8; ++i) {
        unsigned char c = (unsigned char)public_key[i];
        hostname[10 + i] = isalnum(c) ? (char)tolower(c) : '-';
        instance[10 + i] = (char)c;
    }
    hostname[18] = 0; instance[18] = 0;
    mdns_txt_item_t txt[] = { { .key = "key", .value = public_key } };
    if (mdns_init() != ESP_OK || mdns_hostname_set(hostname) != ESP_OK ||
        mdns_service_add(instance, "_vibebuddy", "_tcp", PORT, txt, 1) != ESP_OK)
        puts("MDNS ERROR");
    listener = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP); if (listener < 0) return false;
    int reuse = 1; setsockopt(listener, SOL_SOCKET, SO_REUSEADDR, &reuse, sizeof(reuse));
    struct sockaddr_in address = { .sin_family = AF_INET, .sin_port = htons(PORT), .sin_addr.s_addr = htonl(INADDR_ANY) };
    if (bind(listener, (struct sockaddr *)&address, sizeof(address)) < 0 || listen(listener, 1) < 0 ||
        fcntl(listener, F_SETFL, O_NONBLOCK) < 0) { close(listener); listener = -1; return false; }
    return true;
}

static const char *string_field(const cJSON *root, const char *name)
{ const cJSON *value = cJSON_GetObjectItemCaseSensitive(root, name); return cJSON_IsString(value) ? value->valuestring : NULL; }

bool wifi_link_usb_event(const char *event, const cJSON *root)
{
    if (!bridge) return false;
    if (!strcmp(event, "device.pair")) {
        const char *key = string_field(root, "key");
        const char *name = string_field(root, "name");
        if (!name || strlen(name) > 32 || strchr(name, '\n') || strchr(name, '\r')) name = "";
        if (!key || strlen(key) != KEY_LENGTH || !vb_link_valid_key((const unsigned char *)key, KEY_LENGTH)) { puts("PAIR INVALID"); return true; }
        unsigned i = 0; while (i < pair_count && strcmp(keys[i], key)) ++i;
        if (i == pair_count && pair_count == MAX_PAIRS) { puts("PAIR FULL"); return true; }
        if (i == pair_count) {
            memcpy(keys[i], key, 44); strcpy(names[i], name); ++pair_count;
            if (!store_pairs()) {
                --pair_count;
                puts("LINK SAVE ERROR"); return true;
            }
        } else if (strcmp(names[i], name)) {
            char old[33]; memcpy(old, names[i], sizeof(old)); strcpy(names[i], name);
            if (!store_pairs()) { memcpy(names[i], old, sizeof(old)); puts("LINK SAVE ERROR"); return true; }
        }
        if (!vb_link_pair(bridge, (const unsigned char *)key, KEY_LENGTH, (const unsigned char *)name, strlen(name))) { puts("PAIR INVALID"); return true; }
        printf("PAIRED %s\n", key); return true;
    }
    if (!strcmp(event, "device.pairs")) {
        printf("PAIRS %u\n", pair_count);
        for (unsigned i = 0; i < pair_count; ++i) printf("PAIR %s %s\n", keys[i], names[i]);
        return true;
    }
    if (!strcmp(event, "device.unpair")) {
        const char *key = string_field(root, "key");
        if (!key || strlen(key) != KEY_LENGTH) { puts("PAIR INVALID"); return true; }
        unsigned i = 0; while (i < pair_count && strcmp(keys[i], key)) ++i;
        if (i < pair_count) {
            char old[44], old_name[33]; memcpy(old, keys[i], sizeof(old)); memcpy(old_name, names[i], sizeof(old_name));
            memmove(keys[i], keys[i + 1], (pair_count - i - 1) * sizeof(keys[0]));
            memmove(names[i], names[i + 1], (pair_count - i - 1) * sizeof(names[0])); --pair_count;
            if (!store_pairs()) {
                memmove(keys[i + 1], keys[i], (pair_count - i) * sizeof(keys[0]));
                memmove(names[i + 1], names[i], (pair_count - i) * sizeof(names[0]));
                memcpy(keys[i], old, sizeof(old)); memcpy(names[i], old_name, sizeof(old_name)); ++pair_count;
                puts("LINK SAVE ERROR"); return true;
            }
            vb_link_unpair(bridge, (const unsigned char *)key, KEY_LENGTH, milliseconds(), bridge_output, NULL);
        }
        printf("UNPAIRED %s\n", key); return true;
    }
    if (!strcmp(event, "device.wifi.scan")) {
        if (scan_pending) return true;
        scan_pending = true;
        esp_err_t error = esp_wifi_scan_start(NULL, true);
        if (error != ESP_OK) { scan_pending = false; printf("WIFI SCAN FAILED %d\n", (int)error); }
        return true;
    }
    if (!strcmp(event, "device.wifi.set")) {
        const char *name = string_field(root, "ssid"), *pass = string_field(root, "password");
        if (!name || !pass || !name[0] || strlen(name) > 32 || strlen(pass) > 63) { puts("WIFI INVALID"); return true; }
        if (nvs_set_str(store, "ssid", name) != ESP_OK || nvs_set_str(store, "password", pass) != ESP_OK || nvs_commit(store) != ESP_OK) { puts("LINK SAVE ERROR"); return true; }
        strcpy(ssid, name); strcpy(password, pass);
        wifi_config_t wifi = {0}; memcpy(wifi.sta.ssid, ssid, strlen(ssid)); memcpy(wifi.sta.password, password, strlen(password));
        esp_wifi_disconnect(); esp_wifi_set_config(WIFI_IF_STA, &wifi); esp_wifi_connect(); last_connect_attempt = milliseconds();
        printf("WIFI NETWORK %s\nWIFI CONNECTING %s\n", ssid, ssid); return true;
    }
    if (!strcmp(event, "device.wifi.forget")) {
        esp_err_t a = nvs_erase_key(store, "ssid"), b = nvs_erase_key(store, "password");
        if ((a != ESP_OK && a != ESP_ERR_NVS_NOT_FOUND) || (b != ESP_OK && b != ESP_ERR_NVS_NOT_FOUND) || nvs_commit(store) != ESP_OK) {
            puts("LINK SAVE ERROR"); return true;
        }
        ssid[0] = 0; password[0] = 0;
        esp_wifi_disconnect(); puts("WIFI FORGOTTEN"); return true;
    }
    return false;
}

void wifi_link_usb_activity(void)
{ if (bridge) vb_lan_usb(bridge, milliseconds(), bridge_output, NULL); }

void wifi_link_tick(wifi_line_handler handler, void *context)
{
    if (!bridge || listener < 0) return;
    uint32_t now = milliseconds();
    if (connecting_report) { connecting_report = false; printf("WIFI CONNECTING %s\n", ssid); }
    if (connected_report) { connected_report = false; puts("WIFI CONNECTED"); }
    if (disconnected_report) {
        bool was_connected = disconnect_was_connected; unsigned reason = disconnect_reason;
        disconnected_report = false;
        if (was_connected) puts("WIFI DISCONNECTED");
        else {
            const char *name = wifi_failure_reason(reason);
            if (name) printf("WIFI FAILED %s\n", name);
            else printf("WIFI FAILED reason_%u\n", reason);
        }
    }
    if (scan_report) { scan_report = false; report_scan(); }
    esp_netif_ip_info_t ip;
    bool online = station && esp_netif_get_ip_info(station, &ip) == ESP_OK && ip.ip.addr != 0;
    uint32_t current_ip = online ? ip.ip.addr : 0;
    if (current_ip != last_ip) {
        last_ip = current_ip;
        if (online) {
            char text[64]; snprintf(text, sizeof(text), "WIFI ADDRESS " IPSTR, IP2STR(&ip.ip)); puts(text);
        }
    }
    if (!online && ssid[0] && now - last_connect_attempt >= 10000) {
        printf("WIFI CONNECTING %s\n", ssid); esp_wifi_connect(); last_connect_attempt = now;
    }
    if (online && client < 0) {
        client = accept(listener, NULL, NULL);
        if (client >= 0) {
            ++diagnostics.accepts;
            fcntl(client, F_SETFL, O_NONBLOCK); unsigned char ephemeral[32]; esp_fill_random(ephemeral, sizeof(ephemeral));
            vb_lan_open(bridge, 1, ephemeral, now, bridge_output, NULL); memset(ephemeral, 0, sizeof(ephemeral));
        }
    }
    for (unsigned i = 0; client >= 0 && !close_pending && i < 512; ++i) {
        unsigned char byte; int n = recv(client, &byte, 1, MSG_DONTWAIT);
        if (n == 0) { request_close(CLOSE_PEER_EOF); break; }
        if (n < 0) {
            if (errno != EAGAIN && errno != EWOULDBLOCK) {
                diagnostics.last_recv_errno = errno; request_close(CLOSE_RECEIVE);
            }
            break;
        }
        vb_lan_receive(bridge, 1, &byte, 1, now, bridge_output, NULL);
        if (pending) { pending = false; handler(context, pending_line, false); }
    }
    vb_lan_poll(bridge, now, bridge_output, NULL);
    if (close_pending || (!online && client >= 0)) {
        if (!close_pending) request_close(CLOSE_WIFI);
        vb_lan_closed(bridge, 1, now, bridge_output, NULL); close_client(); close_pending = false; close_reason = CLOSE_NONE; pending = false;
    }
}
