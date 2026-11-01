#ifndef XGGF_AUDIO_H
#define XGGF_AUDIO_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define AUDIO_MAX_FRAME_SAMPLES 320
/* Deep enough to stream losslessly through a natural Wi-Fi window stall (measured near 1.5 s on the
 * target network once link teardowns stopped amplifying them): 1.92 s at 16 kHz, twice that at 8 kHz.
 * Deeper rings measured to starve the Wi-Fi stack's heap on the ESP32-C3. */
#define AUDIO_RING_FRAMES 96

/* Returns whether the line went out on the link it was armed for; false keeps the frame queued for
 * retry from the ring. */
typedef bool (*audio_line_handler)(void *context, const char *line);

bool audio_start(uint32_t rate, uint32_t duration_ms, uint32_t now);
void audio_stop(void);
bool audio_active(void);
uint32_t audio_overruns(void);
size_t audio_push(const int32_t *samples, size_t count, uint32_t now);
void audio_tick(uint32_t now, audio_line_handler handler, void *context);
#endif
