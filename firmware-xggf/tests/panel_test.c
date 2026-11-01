#include "panel.h"
#include <assert.h>
#include <string.h>

int main(void)
{
    uint8_t frame[PANEL_RGB_BYTES] = {0};
    uint8_t out[PANEL_TRANSFER_BYTES];
    assert(!panel_set_pixel(frame, 64, 0, 1, 2, 3));
    assert(!panel_set_pixel(frame, 0, 8, 1, 2, 3));
    assert(!panel_set_pixel(NULL, 0, 0, 1, 2, 3));
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
    return 0;
}
