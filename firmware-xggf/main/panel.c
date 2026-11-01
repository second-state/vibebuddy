#include "panel.h"
#include <string.h>

/* A low fixed cap applies to every pattern, including a future caller error. */
enum { MAX_CHANNEL = 8 };
static unsigned brightness = MAX_CHANNEL;

static bool valid_brightness(unsigned level)
{ return level == 2 || level == 4 || level == 6 || level == 8; }

bool panel_set_brightness(unsigned level)
{
    if (!valid_brightness(level)) return false;
    brightness = level;
    return true;
}

unsigned panel_brightness(void) { return brightness; }

unsigned panel_brightness_next(unsigned level)
{
    if (level == 2) return 4;
    if (level == 4) return 6;
    if (level == 6) return 8;
    return 2;
}

unsigned panel_brightness_or_default(unsigned level)
{ return valid_brightness(level) ? level : MAX_CHANNEL; }

bool panel_set_pixel(uint8_t *frame, unsigned x, unsigned y, uint8_t r, uint8_t g, uint8_t b)
{
    if (!frame || x >= PANEL_WIDTH || y >= PANEL_HEIGHT) return false;
    size_t i = (y * PANEL_WIDTH + x) * 3;
    frame[i] = r;
    frame[i + 1] = g;
    frame[i + 2] = b;
    return true;
}

static void encode_byte(uint8_t *out, uint8_t value)
{
    if (value > MAX_CHANNEL) value = MAX_CHANNEL;
    value = (uint8_t)((value * brightness + MAX_CHANNEL / 2) / MAX_CHANNEL);
    for (unsigned i = 0; i < 4; ++i) {
        uint8_t high = (value & (0x80u >> (2 * i))) ? 0xe : 0x8;
        uint8_t low = (value & (0x40u >> (2 * i))) ? 0xe : 0x8;
        out[i] = (uint8_t)((high << 4) | low);
    }
}

void panel_encode(const uint8_t *frame, uint8_t *transfer)
{
    memset(transfer, 0, PANEL_TRANSFER_BYTES);
    for (unsigned x = 0; x < PANEL_WIDTH; ++x) {
        for (unsigned wire_y = 0; wire_y < PANEL_HEIGHT; ++wire_y) {
            unsigned y = PANEL_HEIGHT - 1 - wire_y;
            const uint8_t *pixel = frame + (y * PANEL_WIDTH + x) * 3;
            uint8_t *encoded = transfer + PANEL_PREFIX_BYTES + (x * PANEL_HEIGHT + wire_y) * 12;
            encode_byte(encoded, pixel[1]);
            encode_byte(encoded + 4, pixel[0]);
            encode_byte(encoded + 8, pixel[2]);
        }
    }
}
