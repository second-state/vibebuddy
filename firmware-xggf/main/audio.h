#ifndef XGGF_AUDIO_H
#define XGGF_AUDIO_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define AUDIO_MAX_FRAME_SAMPLES 320
#define AUDIO_RING_FRAMES 32

typedef void (*audio_line_handler)(void *context, const char *line);

bool audio_start(uint32_t rate, uint32_t duration_ms, uint32_t now);
void audio_stop(void);
bool audio_active(void);
uint32_t audio_overruns(void);
size_t audio_push(const int32_t *samples, size_t count, uint32_t now);
void audio_tick(uint32_t now, audio_line_handler handler, void *context);
#endif
