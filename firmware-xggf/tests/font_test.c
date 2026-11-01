#include "font.h"
#include "panel.h"
#include "status.h"
#include <assert.h>
#include <stdbool.h>
#include <string.h>

static void test_table_covers_printable_ascii(void)
{
    assert(XGGF_FONT_COUNT == 95);
    for (unsigned c = 0x20; c <= 0x7E; ++c) {
        const xggf_glyph *g = xggf_font_glyph((char)c);
        assert(g == &xggf_font[c - 0x20]);
        assert(g->c == (char)c);
    }
    assert(!xggf_font_glyph('\n'));
    assert(!xggf_font_glyph((char)0x7F));
    assert(!xggf_font_glyph((char)0x80));
}

static void test_glyph_bits(void)
{
    for (unsigned i = 0; i < XGGF_FONT_COUNT; ++i) {
        unsigned ink = 0;
        for (unsigned y = 0; y < XGGF_FONT_HEIGHT; ++y) {
            assert(xggf_font[i].rows[y] < 0x20);
            ink |= xggf_font[i].rows[y];
        }
        assert(i == 0 ? ink == 0 : ink != 0);
    }
    /* 'A': arch, sides, crossbar, sides. ':' : two dots centered in column 2. */
    const xggf_glyph *a = xggf_font_glyph('A');
    const uint8_t a_rows[7] = {0x0E, 0x11, 0x11, 0x11, 0x1F, 0x11, 0x11};
    assert(a && !memcmp(a->rows, a_rows, sizeof(a_rows)));
    const xggf_glyph *colon = xggf_font_glyph(':');
    const uint8_t colon_rows[7] = {0x00, 0x04, 0x00, 0x00, 0x04, 0x00, 0x00};
    assert(colon && !memcmp(colon->rows, colon_rows, sizeof(colon_rows)));
    /* 'g' keeps its descender inside the seven-row cell. */
    const xggf_glyph *g = xggf_font_glyph('g');
    assert(g && g->rows[0] == 0x00 && g->rows[5] != 0x00 && g->rows[6] != 0x00);
}

static bool lit(const uint8_t *frame, unsigned x, unsigned y)
{
    const uint8_t *rgb = frame + (y * PANEL_WIDTH + x) * 3;
    return rgb[0] || rgb[1] || rgb[2];
}

static void test_widest_label_fits_the_text_area(void)
{
    status_model m;
    uint8_t first[PANEL_RGB_BYTES], second[PANEL_RGB_BYTES];
    status_init(&m);
    status_event(&m, "device.heartbeat", 0);
    status_toggle_mute(&m, 0);
    status_toggle_mute(&m, 0); /* back to "MUTE OFF", the widest label at eight characters */
    status_render(&m, 0, first);
    status_render(&m, 0, second);
    assert(!memcmp(first, second, sizeof(first)));

    for (unsigned y = 0; y < PANEL_HEIGHT; ++y) {
        for (unsigned x = 10; x <= 12; ++x) assert(!lit(first, x, y)); /* buddy gap */
        for (unsigned x = 60; x < PANEL_WIDTH; ++x) assert(!lit(first, x, y)); /* clip margin */
    }
    for (unsigned x = 13; x < 60; ++x) assert(!lit(first, x, 7)); /* row 7 stays blank */
    bool eighth_char = false;
    for (unsigned x = 49; x < 60; ++x)
        for (unsigned y = 0; y < 7; ++y) eighth_char = eighth_char || lit(first, x, y);
    assert(eighth_char); /* all eight characters rendered, none clipped early */
}

int main(void)
{
    test_table_covers_printable_ascii();
    test_glyph_bits();
    test_widest_label_fits_the_text_area();
    return 0;
}
