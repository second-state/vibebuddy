#include "audio.h"
#include <assert.h>
#include <stdio.h>
#include <string.h>

static char lines[4][960];
static unsigned line_count, frame_count, expected_sequence;
static bool sequence_ok = true;

static void reset(void)
{
    memset(lines, 0, sizeof(lines)); line_count = 0; frame_count = 0; expected_sequence = 0; sequence_ok = true;
}

static void collect(void *context, const char *line)
{
    (void)context;
    if (line_count < 4) snprintf(lines[line_count], sizeof(lines[0]), "%s", line);
    line_count++;
    unsigned sequence;
    if (sscanf(line, "AUDIO FRAME %u", &sequence) == 1) {
        if (sequence != expected_sequence) sequence_ok = false;
        expected_sequence = sequence + 1; frame_count++;
    }
}

static void fidelity(void)
{
    reset();
    assert(!audio_start(4000, 1000, 0));
    assert(!audio_start(8000, 60001, 0));
    assert(audio_start(8000, 20, 0));
    int32_t samples[320] = {0};
    samples[0] = 0x12345678; samples[2] = (int32_t)0xffff0000;
    assert(audio_push(samples, 320, 20) == 320);
    audio_tick(20, collect, NULL);
    assert(!audio_active() && line_count == 3 && frame_count == 1);
    assert(!strcmp(lines[0], "AUDIO BEGIN rate=8000 samples=160"));
    assert(!strncmp(lines[1], "AUDIO FRAME 0 320 1 ", 20));
    char *encoded = strrchr(lines[1], ' ') + 1;
    assert(!strncmp(encoded, "NBL//wA", 7));
    assert(!strcmp(lines[2], "AUDIO END 1 0 320"));
}

static void continuous(void)
{
    reset();
    assert(audio_start(8000, 60000, 0));
    int32_t samples[320];
    for (unsigned frame = 0; frame < 3000; ++frame) {
        for (unsigned i = 0; i < 320; ++i) samples[i] = (int32_t)((frame * 160 + i) << 16);
        audio_push(samples, 320, frame * 20 + 20);
        audio_tick(frame * 20 + 20, collect, NULL);
    }
    audio_tick(60000, collect, NULL);
    assert(!audio_active() && frame_count == 3000 && sequence_ok && audio_overruns() == 0);
}

static void sixteen_kilohertz_and_overruns(void)
{
    reset();
    int32_t samples[640] = {0};
    assert(audio_start(16000, 20, 0));
    audio_push(samples, 640, 20); audio_tick(20, collect, NULL);
    assert(frame_count == 1 && strstr(lines[1], " 640 1 "));
    reset();
    assert(audio_start(8000, 1000, 0));
    for (unsigned i = 0; i < 33; ++i) audio_push(samples, 320, i * 20 + 20);
    expected_sequence = 1;
    audio_tick(0, collect, NULL);
    assert(audio_overruns() == 1 && expected_sequence == 5 && sequence_ok);
    audio_stop();
    for (unsigned now = 1; now <= 10; ++now) audio_tick(now, collect, NULL);
    assert(!audio_active());
}

int main(void)
{
    fidelity(); continuous(); sixteen_kilohertz_and_overruns();
    return 0;
}
