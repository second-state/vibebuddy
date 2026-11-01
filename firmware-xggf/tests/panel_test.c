#include "panel.h"
#include <assert.h>
#include <string.h>

static unsigned decode_byte(const uint8_t *encoded)
{
    unsigned value = 0;
    for (unsigned i = 0; i < 8; ++i) {
        unsigned nibble = (encoded[i / 2] >> (i % 2 ? 0 : 4)) & 15;
        assert(nibble == 8 || nibble == 14);
        value = (value << 1) | (nibble == 14);
    }
    return value;
}

int main(void)
{
    uint8_t frame[PANEL_RGB_BYTES] = {0};
    uint8_t out[PANEL_TRANSFER_BYTES];
    assert(!panel_set_pixel(frame, 64, 0, 1, 2, 3));
    assert(!panel_set_pixel(frame, 0, 8, 1, 2, 3));
    assert(!panel_set_pixel(NULL, 0, 0, 1, 2, 3));
    assert(panel_brightness() == 8);
    assert(panel_set_pixel(frame, 0, 0, 255, 1, 2));
    assert(panel_set_pixel(frame, 63, 7, 0, 0, 8));
    panel_encode(frame, out);
    for (unsigned i = 0; i < PANEL_PREFIX_BYTES; ++i) assert(out[i] == 0);
    for (unsigned i = PANEL_TRANSFER_BYTES - PANEL_SUFFIX_BYTES; i < PANEL_TRANSFER_BYTES; ++i) assert(out[i] == 0);
    /* x=0,y=0 is physical index 7. Green=1, red capped to 8, blue=2. */
    unsigned p = PANEL_PREFIX_BYTES + 7 * 12;
    const uint8_t expected[] = {0x88, 0x88, 0x88, 0x8e, 0x88, 0x88, 0xe8, 0x88, 0x88, 0x88, 0x88, 0xe8};
    assert(memcmp(out + p, expected, sizeof(expected)) == 0);
    /* x=63,y=7 is physical index 504; its blue channel is 8. */
    p = PANEL_PREFIX_BYTES + 504 * 12;
    const uint8_t blue[] = {0x88, 0x88, 0xe8, 0x88};
    assert(memcmp(out + p + 8, blue, sizeof(blue)) == 0);
    for (unsigned i = 0; i < 12; ++i) assert(out[PANEL_PREFIX_BYTES + i] == 0x88);

    for (unsigned i = 0; i < PANEL_RGB_BYTES; ++i) frame[i] = (uint8_t)i;
    const unsigned levels[] = {2, 4, 6, 8};
    for (unsigned i = 0; i < sizeof(levels) / sizeof(levels[0]); ++i) {
        unsigned level = levels[i];
        assert(panel_set_brightness(level));
        panel_encode(frame, out);
        unsigned channel = 0;
        for (unsigned x = 0; x < PANEL_WIDTH; ++x)
            for (unsigned wire_y = 0; wire_y < PANEL_HEIGHT; ++wire_y) {
                const uint8_t *pixel = frame + ((PANEL_HEIGHT - 1 - wire_y) * PANEL_WIDTH + x) * 3;
                const uint8_t inputs[] = {pixel[1], pixel[0], pixel[2]};
                for (unsigned color = 0; color < 3; ++color, ++channel) {
                    unsigned input = inputs[color] > 8 ? 8 : inputs[color];
                    unsigned expected_value = (input * level + 4) / 8;
                    unsigned encoded = decode_byte(out + PANEL_PREFIX_BYTES + channel * 4);
                    assert(encoded == expected_value && encoded <= level && encoded <= 8);
                    if (!input) assert(!encoded);
                }
            }
    }
    assert(panel_brightness_next(2) == 4);
    assert(panel_brightness_next(4) == 6);
    assert(panel_brightness_next(6) == 8);
    assert(panel_brightness_next(8) == 2);
    assert(panel_brightness_or_default(0) == 8);
    assert(panel_brightness_or_default(3) == 8);
    assert(panel_brightness_or_default(9) == 8);
    assert(panel_brightness_or_default(255) == 8);
    assert(panel_brightness_or_default(6) == 6);
    assert(!panel_set_brightness(7) && panel_brightness() == 8);
    return 0;
}
