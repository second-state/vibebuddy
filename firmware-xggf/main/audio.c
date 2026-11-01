#include "audio.h"
#include <stdio.h>
#include <string.h>
#define AUDIO_LOCK()
#define AUDIO_UNLOCK()

static uint8_t ring[AUDIO_RING_FRAMES][AUDIO_MAX_FRAME_SAMPLES * 2];
static uint32_t sequences[AUDIO_RING_FRAMES];
static uint32_t rate, duration_ms, started_at, next_sequence, overrun_count, raw_slots;
static size_t ring_head, ring_count;
static int16_t partial[AUDIO_MAX_FRAME_SAMPLES];
static size_t partial_count;
static bool active, stopping, begun, discontinuity;

static size_t base64_encode(const uint8_t *input, size_t length, char *output)
{
    static const char alphabet[] = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    size_t out = 0;
    for (size_t i = 0; i < length; i += 3) {
        uint32_t value = (uint32_t)input[i] << 16;
        if (i + 1 < length) value |= (uint32_t)input[i + 1] << 8;
        if (i + 2 < length) value |= input[i + 2];
        output[out++] = alphabet[(value >> 18) & 63];
        output[out++] = alphabet[(value >> 12) & 63];
        output[out++] = i + 1 < length ? alphabet[(value >> 6) & 63] : '=';
        output[out++] = i + 2 < length ? alphabet[value & 63] : '=';
    }
    output[out] = 0;
    return out;
}

bool audio_start(uint32_t new_rate, uint32_t new_duration_ms, uint32_t now)
{
    AUDIO_LOCK();
    if (active || (new_rate != 8000 && new_rate != 16000) || new_duration_ms == 0 || new_duration_ms > 60000) {
        AUDIO_UNLOCK(); return false;
    }
    rate = new_rate; duration_ms = new_duration_ms; started_at = now;
    next_sequence = 0; overrun_count = 0; raw_slots = 0; ring_head = 0; ring_count = 0; partial_count = 0;
    stopping = false; begun = false; discontinuity = false; active = true;
    AUDIO_UNLOCK();
    return true;
}

void audio_stop(void)
{
    AUDIO_LOCK(); if (active) stopping = true; AUDIO_UNLOCK();
}

bool audio_active(void)
{
    AUDIO_LOCK(); bool result = active; AUDIO_UNLOCK(); return result;
}

uint32_t audio_overruns(void)
{
    AUDIO_LOCK(); uint32_t result = overrun_count; AUDIO_UNLOCK(); return result;
}

size_t audio_push(const int32_t *samples, size_t count, uint32_t now)
{
    AUDIO_LOCK();
    if (!active || stopping || !samples) { AUDIO_UNLOCK(); return 0; }
    raw_slots += count;
    size_t frame_samples = rate / 50;
    size_t allowed_frames = (uint32_t)(now - started_at) / 20;
    if (next_sequence >= allowed_frames) { AUDIO_UNLOCK(); return 0; }
    size_t remaining_samples = (allowed_frames - next_sequence) * frame_samples - partial_count;
    if (count > remaining_samples * 2) count = remaining_samples * 2;
    size_t consumed = 0;
    while (consumed < count) {
        size_t take = frame_samples - partial_count;
        if (take > (count - consumed) / 2) take = (count - consumed) / 2;
        if (!take) break;
        for (size_t i = 0; i < take; ++i)
            partial[partial_count + i] = (int16_t)(samples[consumed + i * 2] >> 16);
        partial_count += take; consumed += take * 2;
        if (partial_count != frame_samples) continue;
        if (ring_count == AUDIO_RING_FRAMES) {
            ring_head = (ring_head + 1) % AUDIO_RING_FRAMES;
            --ring_count; ++overrun_count; discontinuity = true;
        }
        size_t slot = (ring_head + ring_count) % AUDIO_RING_FRAMES;
        for (size_t i = 0; i < frame_samples; ++i) {
            ring[slot][i * 2] = (uint8_t)partial[i];
            ring[slot][i * 2 + 1] = (uint8_t)(partial[i] >> 8);
        }
        sequences[slot] = next_sequence++;
        ring_count++; partial_count = 0;
    }
    AUDIO_UNLOCK();
    return consumed;
}

void audio_tick(uint32_t now, audio_line_handler handler, void *context)
{
    if (!handler) return;
    char line[960];
    uint32_t begin_rate = 0;
    AUDIO_LOCK();
    if (!active) { AUDIO_UNLOCK(); return; }
    if (!begun) { begun = true; begin_rate = rate; }
    if ((uint32_t)(now - started_at) >= duration_ms) stopping = true;
    AUDIO_UNLOCK();
    if (begin_rate) {
        snprintf(line, sizeof(line), "AUDIO BEGIN rate=%u samples=%u", (unsigned)begin_rate, (unsigned)(begin_rate / 50));
        handler(context, line);
    }
    for (unsigned emitted = 0; emitted < 4; ++emitted) {
        uint8_t pcm[AUDIO_MAX_FRAME_SAMPLES * 2];
        uint32_t sequence;
        size_t bytes;
        unsigned flags;
        AUDIO_LOCK();
        if (!active || !ring_count) { AUDIO_UNLOCK(); break; }
        bytes = rate / 50 * 2;
        memcpy(pcm, ring[ring_head], bytes);
        sequence = sequences[ring_head];
        flags = discontinuity ? 2 : 0;
        if (stopping && ring_count == 1) flags |= 1;
        discontinuity = false;
        ring_head = (ring_head + 1) % AUDIO_RING_FRAMES; --ring_count;
        AUDIO_UNLOCK();
        char encoded[AUDIO_MAX_FRAME_SAMPLES * 2 * 4 / 3 + 5];
        base64_encode(pcm, bytes, encoded);
        snprintf(line, sizeof(line), "AUDIO FRAME %u %u %u %s", (unsigned)sequence, (unsigned)bytes, flags, encoded);
        handler(context, line);
    }
    uint32_t end_sequence, end_overruns, end_slots;
    bool finished = false;
    AUDIO_LOCK();
    if (active && stopping && !ring_count) {
        end_sequence = next_sequence; end_overruns = overrun_count; end_slots = raw_slots;
        active = false; partial_count = 0; finished = true;
    }
    AUDIO_UNLOCK();
    if (finished) {
        snprintf(line, sizeof(line), "AUDIO END %u %u %u", (unsigned)end_sequence, (unsigned)end_overruns, (unsigned)end_slots);
        handler(context, line);
    }
}
