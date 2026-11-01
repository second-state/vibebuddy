#ifndef XGGF_PANEL_H
#define XGGF_PANEL_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

enum { PANEL_WIDTH = 64, PANEL_HEIGHT = 8, PANEL_PIXELS = 512,
       PANEL_RGB_BYTES = PANEL_PIXELS * 3, PANEL_PREFIX_BYTES = 60,
       PANEL_SUFFIX_BYTES = 120, PANEL_TRANSFER_BYTES = PANEL_PREFIX_BYTES + PANEL_PIXELS * 12 + PANEL_SUFFIX_BYTES };

/* Logical pixels are row-major, RGB, with (0, 0) at the top left. */
bool panel_set_pixel(uint8_t *frame, unsigned x, unsigned y, uint8_t r, uint8_t g, uint8_t b);
/* Writes the inferred column-major, inverted-Y, GRB WS2812 waveform. */
void panel_encode(const uint8_t *frame, uint8_t *transfer);

#endif
