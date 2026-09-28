#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "agent_audio.h"
#include "agent_build_stamp.h"
#include "agent_buttons.h"
#include "agent_display.h"
#include "agent_leisure.h"
#include "agent_pomodoro.h"
#include "agent_tally.h"
#include "agent_voice_pack.h"
#include "agent_voices.h"
#include "cJSON.h"
#include "driver/uart.h"
#include "esp_app_desc.h"
#include "driver/usb_serial_jtag.h"
#include "esp_check.h"
#include "esp_random.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

#define MAX_LINE_BYTES 1024
#define LINE_BUFFER_BYTES (MAX_LINE_BYTES + 2)
#define IO_BUFFER_BYTES 256
/// Receive ring buffer of the UART driver: one voice pack line is close to 1 KB, so leave
/// room for several lines. Back-to-back long lines get mangled on the UART bridge; the Mac
/// solves that by pacing its sends to the line rate, and a control experiment polling the
/// FIFO directly cleared the device-side driver and interrupt path.
#define UART_RX_BUFFER_BYTES 4096

static bool ready_scheduled;
static TickType_t ready_deadline;
/// With no message for longer than this, the link to the Mac counts as lost.
#define LINK_TIMEOUT_MS 15000
static TickType_t last_message_tick;
static bool link_lost;
/// The level, backlight state and hour last reported to the Mac; each is reported as a line only when it changes.
static agent_leisure_tier_t reported_tier = AGENT_LEISURE_ALERT;
static bool reported_lights_out;
static int reported_hour = -1;

// Both outputs are only diagnostic channels, and neither may hold up the main loop. When
// the BOX's UART bridge is used, no host drains the USB Serial/JTAG side, so once its tx
// ring buffer fills any wait is forever: the main task stalls, the screen freezes, link-loss
// detection dies with it, and the Mac's heartbeat still gets written to the port, so
// neither side notices the device is gone. If a write doesn't fit, drop that piece.
static void transport_write_all(const char *data, size_t length) {
  uart_write_bytes(UART_NUM_0, data, length);
  usb_serial_jtag_write_bytes(data, length, 0);
}

static void transport_write_literal(const char *text) {
  transport_write_all(text, strlen(text));
}

static void transport_write_value_line(const char *label, const char *value) {
  transport_write_literal(label);
  while (*value != '\0') {
    char output = (*value == '\r' || *value == '\n') ? ' ' : *value;
    transport_write_all(&output, 1);
    value++;
  }
  transport_write_literal("\n");
}

static void handle_voice_event(const cJSON *message, const char *event);
static void announce_state(void);

static uint32_t clock_ms(void) {
  return xTaskGetTickCount() * (uint32_t)portTICK_PERIOD_MS;
}

static void write_shot_line(const char *line) {
  transport_write_literal(line);
  transport_write_literal("\n");
}

static const char *mode_name(agent_mode_t mode) {
  switch (mode) {
    case AGENT_MODE_POMODORO:
      return "POMODORO";
    case AGENT_MODE_LEISURE:
      return "LEISURE";
    default:
      return "DUTY";
  }
}

static void set_mode(agent_mode_t mode) {
  if (agent_display_mode() == mode) {
    return;
  }
  agent_display_set_mode(mode);
  transport_write_value_line("MODE ", mode_name(mode));
}

static void set_link_lost(bool lost) {
  link_lost = lost;
  agent_display_set_link_lost(lost);
}

static void report_pomodoro(const char *what) {
  transport_write_value_line("POMODORO ", what);
}

/// Mute: long-press K2 to toggle; not persisted. Mute it for a meeting and forget, and the
/// device would stay silent for days; coming back with sound after a restart is safer than
/// remembering, and the MUTE badge on screen is the reminder.
static bool muted;

/// Every voice line goes out through here; when muted it only logs a line.
static void play_prompt(agent_audio_prompt_t prompt, const char *label) {
  if (muted) {
    transport_write_value_line("AUDIO MUTED ", label);
    return;
  }
  if (agent_audio_play(prompt) != ESP_OK) {
    transport_write_literal("AUDIO ERROR\n");
  } else {
    transport_write_value_line("AUDIO QUEUED ", label);
  }
}

/// Volume is the device's own fact and the app's slider is just a remote: report a line after a change, and on hello too.
static void announce_volume(void) {
  char text[24];
  snprintf(text, sizeof(text), "VOLUME %u\n", agent_audio_volume() % 1000u);
  transport_write_literal(text);
}

/// When today's record changes, save it once and report a line to the Mac. It happens a few times a day; NVS doesn't mind.
static void save_tally(void) {
  agent_pomodoro_tally_t tally;
  agent_pomodoro_tally(&tally);
  char text[48];
  snprintf(text, sizeof(text), "POMODORO TODAY %u %uS DAY %u\n",
           tally.completed % 10000u, (unsigned)(tally.focus_s % 1000000u),
           (unsigned)(tally.day % 100000000u));
  transport_write_literal(text);
  if (agent_tally_save(&tally) != ESP_OK) {
    transport_write_literal("TALLY SAVE ERROR\n");
  }
}

/// Brings the pomodoro to the front; if it is already there, just redraws.
static void show_pomodoro(void) {
  if (agent_display_mode() == AGENT_MODE_POMODORO) {
    agent_display_refresh();
  } else {
    set_mode(AGENT_MODE_POMODORO);
  }
}

/// Each of the three keys does one thing regardless of mode: K0 is the pomodoro key, K1
/// switches between duty and pomodoro (long press goes to leisure), and K2 asks the Mac to
/// open the source. In leisure mode any key first calls the buddy back to duty and then does
/// its own job: leisure hides nothing that needs a look first.
static void on_button(agent_button_event_t event) {
  uint32_t now = clock_ms();
  agent_mode_t mode_before = agent_display_mode();
  agent_leisure_note_activity(now);
  if (mode_before == AGENT_MODE_LEISURE) {
    (void)agent_leisure_tick(now);
    set_mode(AGENT_MODE_DUTY);
  }

  if (event == AGENT_BUTTON_K2_SHORT) {
    transport_write_literal(
        "{\"version\":1,\"event\":\"button\",\"button\":\"K2\",\"action\":\"press\"}\n");
    return;
  }
  if (event == AGENT_BUTTON_K2_LONG) {
    muted = !muted;
    agent_display_set_muted(muted);
    transport_write_literal(muted ? "MUTE ON\n" : "MUTE OFF\n");
    return;
  }
  if (event == AGENT_BUTTON_K1_SHORT) {
    set_mode(mode_before == AGENT_MODE_POMODORO ? AGENT_MODE_DUTY
                                                : AGENT_MODE_POMODORO);
    return;
  }
  if (event == AGENT_BUTTON_K1_LONG) {
    agent_leisure_force_bored(now);
    (void)agent_leisure_tick(now);
    set_mode(AGENT_MODE_LEISURE);
    return;
  }

  agent_pomodoro_view_t before;
  agent_pomodoro_view(now, &before);
  bool break_phase = before.phase == AGENT_POMODORO_BREAK;
  if (event == AGENT_BUTTON_K0_LONG) {
    if (agent_pomodoro_is_idle(&before)) {
      return;
    }
    agent_pomodoro_stop();
    report_pomodoro(break_phase && before.run == AGENT_POMODORO_PENDING
                        ? "BREAK SKIPPED"
                        : "STOPPED");
    agent_display_refresh();
    return;
  }
  agent_pomodoro_toggle(now);
  if (before.run == AGENT_POMODORO_PENDING) {
    report_pomodoro(break_phase ? "BREAK START" : "FOCUS START");
    // Starting a phase brings the pomodoro to the front: the ring starting to move is the feedback.
    show_pomodoro();
    return;
  }
  // Pause and resume don't change scenes; the badge in the top right of the buddy scene flashes along.
  report_pomodoro(before.run == AGENT_POMODORO_PAUSED ? "RESUMED" : "PAUSED");
  agent_display_refresh();
}

/// A phase end is an edge consumed once: play the voice once and bring the pomodoro to the
/// front, since this is exactly when the user should glance at it. The next phase waits to
/// start until the user presses K0.
static void handle_pomodoro_transition(agent_pomodoro_transition_t transition) {
  if (transition == AGENT_POMODORO_NOTHING) {
    return;
  }
  bool focus_ended = transition == AGENT_POMODORO_FOCUS_ENDED;
  report_pomodoro(focus_ended ? "FOCUS END" : "BREAK END");
  if (focus_ended) {
    save_tally();
  }
  play_prompt(focus_ended ? AGENT_AUDIO_FOCUS_DONE : AGENT_AUDIO_BREAK_DONE,
              focus_ended ? "FOCUS_DONE" : "BREAK_DONE");
  agent_display_pomodoro_ended();
  show_pomodoro();
}

/// Boredom accumulates on state, not on messages: agents at work, a running pomodoro or a
/// lost link are not idle. After duty has been idle long enough, go to leisure; come back
/// the moment something happens. In pomodoro mode, a still 25:00 waiting to start is as dull
/// as idle duty and leaves after five minutes; a pause is the user stopping there on
/// purpose, so only after half an hour is the user assumed gone. Either way it returns to
/// duty first and then drifts into leisure naturally.
static void tend_leisure(void) {
  uint32_t now = clock_ms();
  agent_pomodoro_view_t pomodoro;
  agent_pomodoro_view(now, &pomodoro);
  if (!agent_display_agent_idle() ||
      pomodoro.run == AGENT_POMODORO_RUNNING || link_lost) {
    agent_leisure_note_activity(now);
  }

  agent_mode_t mode = agent_display_mode();
  bool changed = agent_leisure_tick(now);
  agent_leisure_view_t leisure;
  agent_leisure_view(now, &leisure);
  // Report the level by difference, not by "did it change this time": the wake path advances the director elsewhere first.
  if (leisure.tier != reported_tier) {
    reported_tier = leisure.tier;
    transport_write_value_line("LEISURE ",
                               agent_leisure_tier_name(leisure.tier));
  } else if (changed && mode == AGENT_MODE_LEISURE) {
    transport_write_value_line("LEISURE SKIT ",
                               agent_leisure_skit_name(leisure.skit));
  }
  if (leisure.lights_out != reported_lights_out) {
    reported_lights_out = leisure.lights_out;
    transport_write_literal(leisure.lights_out ? "LEISURE LIGHTS OUT\n"
                                               : "LEISURE LIGHTS ON\n");
  }

  if (mode == AGENT_MODE_DUTY && leisure.tier != AGENT_LEISURE_ALERT) {
    set_mode(AGENT_MODE_LEISURE);
  } else if (mode == AGENT_MODE_LEISURE &&
             leisure.tier == AGENT_LEISURE_ALERT) {
    set_mode(AGENT_MODE_DUTY);
  } else if (mode == AGENT_MODE_POMODORO) {
    bool pending = pomodoro.run == AGENT_POMODORO_PENDING;
    if (leisure.tier == AGENT_LEISURE_SLEEPY ||
        (pending && leisure.tier == AGENT_LEISURE_BORED)) {
      set_mode(AGENT_MODE_DUTY);
    }
  }
}

static agent_display_state_t task_state(const char *status) {
  if (strcmp(status, "input_required") == 0) {
    return AGENT_DISPLAY_INPUT_REQUIRED;
  }
  if (strcmp(status, "done") == 0) {
    return AGENT_DISPLAY_DONE;
  }
  if (strcmp(status, "failed") == 0) {
    return AGENT_DISPLAY_FAILED;
  }
  return AGENT_DISPLAY_WORKING;
}

static size_t parse_tasks(const cJSON *message, agent_display_task_t *tasks) {
  const cJSON *task_list = cJSON_GetObjectItemCaseSensitive(message, "tasks");
  if (!cJSON_IsArray(task_list)) {
    return 0;
  }

  size_t count = 0;
  const cJSON *item;
  cJSON_ArrayForEach(item, task_list) {
    if (count == AGENT_DISPLAY_MAX_TASKS) {
      break;
    }
    const cJSON *title = cJSON_GetObjectItemCaseSensitive(item, "title");
    const cJSON *status = cJSON_GetObjectItemCaseSensitive(item, "status");
    if (!cJSON_IsString(title) || !cJSON_IsString(status)) {
      continue;
    }
    const cJSON *elapsed = cJSON_GetObjectItemCaseSensitive(item, "elapsed_s");
    const cJSON *project = cJSON_GetObjectItemCaseSensitive(item, "project");
    tasks[count].title = title->valuestring;
    tasks[count].state = task_state(status->valuestring);
    tasks[count].elapsed_s = cJSON_IsNumber(elapsed) ? (int)elapsed->valuedouble : 0;
    tasks[count].project = cJSON_IsString(project) ? project->valuestring : NULL;
    count++;
  }
  return count;
}

/// Today's stats come with every state event, and the idle screen rotates through them.
///
/// Taking them only from `agent.idle` isn't enough: after `task.done` the device returns to
/// idle on its own, and that is exactly when the user glances over, so the cached stats
/// must already include the task just finished.
static void parse_stats(const cJSON *message) {
  const cJSON *stat_list = cJSON_GetObjectItemCaseSensitive(message, "stats");
  if (!cJSON_IsArray(stat_list)) {
    return;
  }

  const char *lines[AGENT_DISPLAY_MAX_STATS];
  size_t count = 0;
  const cJSON *item;
  cJSON_ArrayForEach(item, stat_list) {
    if (count == AGENT_DISPLAY_MAX_STATS) {
      break;
    }
    if (!cJSON_IsString(item)) {
      continue;
    }
    lines[count++] = item->valuestring;
  }
  agent_display_set_stats(lines, count);

  // The number in the "7 DONE" line: in leisure it decides whether the buddy is tired or bored.
  unsigned done = 0;
  for (size_t index = 0; index < count; index++) {
    if (strstr(lines[index], "DONE") != NULL) {
      done = (unsigned)strtoul(lines[index], NULL, 10);
      break;
    }
  }
  agent_leisure_set_done_count(done);
}

static void show_event(const cJSON *message, const char *event,
                       const char *title) {
  agent_display_state_t state;
  agent_display_task_t tasks[AGENT_DISPLAY_MAX_TASKS];
  size_t task_count = parse_tasks(message, tasks);
  agent_audio_prompt_t prompt = AGENT_AUDIO_INPUT_REQUIRED;
  bool should_play = false;
  const char *prompt_label = NULL;
  const char *state_label;
  if (strcmp(event, "task.start") == 0) {
    state = AGENT_DISPLAY_WORKING;
    state_label = "WORKING";
    ready_scheduled = false;
  } else if (strcmp(event, "agent.idle") == 0) {
    state = AGENT_DISPLAY_IDLE;
    state_label = "READY";
    ready_scheduled = false;
  } else if (strcmp(event, "agent.input_required") == 0) {
    state = AGENT_DISPLAY_INPUT_REQUIRED;
    prompt = AGENT_AUDIO_INPUT_REQUIRED;
    prompt_label = "INPUT_REQUIRED";
    should_play = true;
    state_label = "INPUT REQUIRED";
    ready_scheduled = false;
  } else if (strcmp(event, "task.done") == 0) {
    state = AGENT_DISPLAY_DONE;
    prompt = AGENT_AUDIO_DONE;
    prompt_label = "DONE";
    should_play = true;
    state_label = "DONE";
    ready_scheduled = true;
    ready_deadline = xTaskGetTickCount() + pdMS_TO_TICKS(5000);
  } else if (strcmp(event, "task.error") == 0 ||
             strcmp(event, "agent.blocked") == 0) {
    state = AGENT_DISPLAY_FAILED;
    prompt = AGENT_AUDIO_FAILED;
    prompt_label = "FAILED";
    should_play = true;
    state_label = "FAILED";
    ready_scheduled = false;
  } else {
    return;
  }

  const cJSON *suppress_audio =
      cJSON_GetObjectItemCaseSensitive(message, "suppress_audio");
  if (cJSON_IsTrue(suppress_audio)) {
    should_play = false;
    prompt_label = NULL;
  }

  const cJSON *announcement =
      cJSON_GetObjectItemCaseSensitive(message, "announcement");
  if (cJSON_IsString(announcement)) {
    if (strcmp(announcement->valuestring, "done") == 0) {
      prompt = AGENT_AUDIO_DONE;
      prompt_label = "DONE";
      should_play = true;
    } else if (strcmp(announcement->valuestring, "failed") == 0) {
      prompt = AGENT_AUDIO_FAILED;
      prompt_label = "FAILED";
      should_play = true;
    }
  }

  if (agent_display_show_tasks(state, title, tasks, task_count) != ESP_OK) {
    transport_write_literal("DISPLAY ERROR\n");
  } else {
    transport_write_value_line("DISPLAY STATE ", state_label);
  }
  if (should_play) {
    play_prompt(prompt, prompt_label);
  }
}

static void handle_line(char *line, size_t length) {
  if (length == 0) {
    return;
  }

  cJSON *message = cJSON_ParseWithLength(line, length);
  if (message == NULL) {
    transport_write_literal("ERROR invalid_json\n");
    return;
  }

  const cJSON *version = cJSON_GetObjectItemCaseSensitive(message, "version");
  const cJSON *event = cJSON_GetObjectItemCaseSensitive(message, "event");
  const cJSON *title = cJSON_GetObjectItemCaseSensitive(message, "title");

  if (!cJSON_IsObject(message) || !cJSON_IsNumber(version) ||
      !cJSON_IsString(event) || event->valuestring[0] == '\0' ||
      (title != NULL && !cJSON_IsString(title))) {
    transport_write_literal("ERROR invalid_message\n");
    cJSON_Delete(message);
    return;
  }

  if (version->valuedouble != 1.0) {
    transport_write_literal("ERROR unsupported_version\n");
    cJSON_Delete(message);
    return;
  }

  // The heartbeat only proves the link is alive; it isn't shown or echoed, since a diagnostic
  // line every 5 seconds would drown the log. It also carries the Mac's build stamp: the
  // device may restart at any time, and a one-off handshake would be lost.
  if (strcmp(event->valuestring, "device.heartbeat") == 0) {
    const cJSON *build = cJSON_GetObjectItemCaseSensitive(message, "build");
    if (cJSON_IsString(build)) {
      agent_display_set_daemon_build(build->valuestring);
    }
    // The local hour also comes with the heartbeat: the device has no clock, so day and night are whatever the Mac says.
    const cJSON *hour = cJSON_GetObjectItemCaseSensitive(message, "hour");
    if (cJSON_IsNumber(hour) && (int)hour->valuedouble != reported_hour) {
      reported_hour = (int)hour->valuedouble;
      agent_leisure_set_hour(reported_hour);
      char text[24];
      snprintf(text, sizeof(text), "CLOCK HOUR %d\n", reported_hour % 100);
      transport_write_literal(text);
    }
    // The local date also comes with the heartbeat: the pomodoro's daily record resets on it.
    const cJSON *day = cJSON_GetObjectItemCaseSensitive(message, "day");
    if (cJSON_IsNumber(day) && day->valuedouble > 0 &&
        agent_pomodoro_set_day((uint32_t)day->valuedouble)) {
      save_tally();
      agent_display_refresh();
    }
    cJSON_Delete(message);
    return;
  }

  // A screenshot is a debugging action: it isn't agent activity and doesn't wake leisure.
  if (strcmp(event->valuestring, "device.screenshot") == 0) {
    agent_display_dump(write_shot_line);
    cJSON_Delete(message);
    return;
  }

  // The Mac asks when it first connects: mode, firmware build and voice are only reported
  // at boot or on change, and the daemon restarts more often than the device, so without
  // asking it would never know.
  if (strcmp(event->valuestring, "device.hello") == 0) {
    announce_state();
    cJSON_Delete(message);
    return;
  }

  // Blink-to-identify and voice pack writes are the app operating the device itself, so they aren't agent activity either.
  if (strcmp(event->valuestring, "device.identify") == 0) {
    agent_display_identify();
    transport_write_literal("IDENTIFY\n");
    cJSON_Delete(message);
    return;
  }
  // Volume: the app's slider lands here, goes to the codec and is saved to NVS; without a
  // level it is just a query. The preview goes through play_prompt, so it is silent when
  // muted, same rule as every other announcement.
  if (strcmp(event->valuestring, "device.volume") == 0) {
    const cJSON *level = cJSON_GetObjectItemCaseSensitive(message, "level");
    if (cJSON_IsNumber(level)) {
      unsigned wanted = level->valuedouble < 0 ? 0u : (unsigned)level->valuedouble;
      if (agent_audio_set_volume(wanted) != ESP_OK) {
        transport_write_literal("VOLUME ERROR\n");
      }
    }
    announce_volume();
    if (cJSON_IsTrue(cJSON_GetObjectItemCaseSensitive(message, "preview"))) {
      play_prompt(AGENT_AUDIO_DONE, "DONE");
    }
    cJSON_Delete(message);
    return;
  }
  if (strncmp(event->valuestring, "voice.", 6) == 0) {
    handle_voice_event(message, event->valuestring);
    cJSON_Delete(message);
    return;
  }
  // Link self-test: send the received string's length and CRC back to the Mac to check for corrupted serial bytes.
  if (strcmp(event->valuestring, "device.echo") == 0) {
    const cJSON *data = cJSON_GetObjectItemCaseSensitive(message, "data");
    if (cJSON_IsString(data)) {
      size_t length = strlen(data->valuestring);
      uint32_t crc = agent_voice_pack_crc32(
          0, (const uint8_t *)data->valuestring, length);
      char reply[96];
      snprintf(reply, sizeof(reply),
               "{\"version\":1,\"event\":\"echo\",\"length\":%u,\"crc\":%lu}\n",
               (unsigned)length, (unsigned long)crc);
      transport_write_literal(reply);
      // Echo the line back verbatim; the Mac compares byte by byte to see what the port actually received.
      transport_write_value_line("ECHO ", data->valuestring);
    }
    cJSON_Delete(message);
    return;
  }

  // As soon as an agent does something, the buddy comes straight back to duty.
  agent_leisure_note_activity(clock_ms());
  if (agent_display_mode() == AGENT_MODE_LEISURE) {
    (void)agent_leisure_tick(clock_ms());
    set_mode(AGENT_MODE_DUTY);
  }

  parse_stats(message);
  transport_write_value_line("EVENT ", event->valuestring);
  if (title != NULL) {
    transport_write_value_line("TITLE ", title->valuestring);
  }
  show_event(message, event->valuestring,
             title == NULL ? NULL : title->valuestring);

  cJSON_Delete(message);
}

static char announced_build[48];

/// Reports the device's whole static state: firmware build, mode, voice and volume. Both
/// boot and hello go through here.
static void announce_state(void) {
  transport_write_value_line("DISPLAY READY BUILD ", announced_build);
  transport_write_value_line("MODE ", mode_name(agent_display_mode()));
  transport_write_value_line("VOICES ", agent_voices_current_id());
  announce_volume();
}

/// Voice pack write acknowledgements are all JSON lines: the Mac does stop-and-wait flow control by sequence number, which diagnostic lines can't support.
static void voice_reply(const char *event, int32_t seq, const char *detail) {
  char line[160];
  if (strcmp(event, "voice.written") == 0) {
    snprintf(line, sizeof(line),
             "{\"version\":1,\"event\":\"voice.written\",\"voice\":\"%s\"}\n",
             detail);
  } else if (strcmp(event, "voice.error") == 0) {
    snprintf(line, sizeof(line),
             "{\"version\":1,\"event\":\"voice.error\",\"seq\":%ld,"
             "\"message\":\"%s\"}\n",
             (long)seq, detail);
  } else {
    snprintf(line, sizeof(line), "{\"version\":1,\"event\":\"%s\",\"seq\":%ld}\n",
             event, (long)seq);
  }
  transport_write_literal(line);
}

static void handle_voice_event(const cJSON *message, const char *event) {
  if (strcmp(event, "voice.begin") == 0) {
    const cJSON *size = cJSON_GetObjectItemCaseSensitive(message, "size");
    esp_err_t result = cJSON_IsNumber(size)
                           ? agent_voices_begin((uint32_t)size->valuedouble)
                           : ESP_ERR_INVALID_ARG;
    if (result != ESP_OK) {
      voice_reply("voice.error", -1, esp_err_to_name(result));
      return;
    }
    voice_reply("voice.ready", -1, NULL);
    return;
  }
  if (strcmp(event, "voice.chunk") == 0) {
    const cJSON *seq = cJSON_GetObjectItemCaseSensitive(message, "seq");
    const cJSON *data = cJSON_GetObjectItemCaseSensitive(message, "data");
    const cJSON *crc = cJSON_GetObjectItemCaseSensitive(message, "crc");
    if (!cJSON_IsNumber(seq) || !cJSON_IsString(data) || !cJSON_IsNumber(crc)) {
      voice_reply("voice.error", -1, "invalid chunk");
      agent_voices_abort();
      return;
    }
    esp_err_t result = agent_voices_chunk(
        (uint32_t)seq->valuedouble, data->valuestring,
        strlen(data->valuestring), (uint32_t)crc->valuedouble);
    if (result != ESP_OK) {
      voice_reply("voice.error", (int32_t)seq->valuedouble,
                  esp_err_to_name(result));
      agent_voices_abort();
      return;
    }
    voice_reply("voice.ack", (int32_t)seq->valuedouble, NULL);
    return;
  }
  if (strcmp(event, "voice.end") == 0) {
    esp_err_t result = agent_voices_end();
    if (result != ESP_OK) {
      voice_reply("voice.error", -1, esp_err_to_name(result));
      agent_voices_abort();
      return;
    }
    voice_reply("voice.written", -1, agent_voices_current_id());
    transport_write_value_line("VOICES ", agent_voices_current_id());
    // Say a line in the new voice when done: over the bridge a write takes several minutes,
    // and the user may not be watching the app for the preview button; the box speaking up
    // is the most direct "done" (on 2026-09-22 a colleague finished a write and thought
    // there was no sound).
    play_prompt(AGENT_AUDIO_DONE, "DONE");
    return;
  }
  voice_reply("voice.error", -1, "unknown voice event");
}

/// The version is the git description in esp_app_desc; the time is not its __TIME__, which
/// only updates when esp_app_desc.c is recompiled and sticks at the last full build after
/// incremental builds. AGENT_BUILD_STAMP is regenerated by main/CMakeLists.txt on every
/// build, in the same format as the Mac side, since the two lines are compared verbatim.
static void describe_firmware_build(char *out, size_t size) {
  const esp_app_desc_t *desc = esp_app_get_description();
  snprintf(out, size, "%.24s %.16s", desc->version, AGENT_BUILD_STAMP);
}

void app_main(void) {
  ESP_ERROR_CHECK(
      uart_driver_install(UART_NUM_0, UART_RX_BUFFER_BYTES, 0, 0, NULL, 0));

  usb_serial_jtag_driver_config_t usb_config = {
      .rx_buffer_size = LINE_BUFFER_BYTES,
      .tx_buffer_size = MAX_LINE_BYTES + 1,
  };
  ESP_ERROR_CHECK(usb_serial_jtag_driver_install(&usb_config));

  describe_firmware_build(announced_build, sizeof(announced_build));

  agent_pomodoro_init();
  agent_leisure_init(esp_random(), clock_ms());
  // Restore yesterday's record too: whether the day changed is only known once a heartbeat brings the date.
  agent_pomodoro_tally_t tally;
  if (agent_tally_init() == ESP_OK && agent_tally_load(&tally) == ESP_OK) {
    agent_pomodoro_restore_tally(&tally);
    char text[48];
    snprintf(text, sizeof(text), "TALLY LOADED %u %uS DAY %u\n",
             tally.completed % 10000u, (unsigned)(tally.focus_s % 1000000u),
             (unsigned)(tally.day % 100000000u));
    transport_write_literal(text);
  } else {
    transport_write_literal("TALLY LOAD ERROR\n");
  }
  if (agent_display_init() == ESP_OK) {
    agent_display_set_firmware_build(announced_build);
    transport_write_value_line("DISPLAY READY BUILD ", announced_build);
  } else {
    transport_write_literal("DISPLAY ERROR\n");
  }
  if (agent_voices_init() == ESP_OK) {
    transport_write_value_line("VOICES ", agent_voices_current_id());
  } else {
    transport_write_literal("VOICES NO PARTITION\n");
  }
  if (agent_audio_init() == ESP_OK) {
    transport_write_literal("AUDIO READY\n");
    transport_write_value_line("AUDIO CODEC ", agent_audio_status());
    announce_volume();
  } else {
    transport_write_value_line("AUDIO ERROR ", agent_audio_status());
  }
  if (agent_buttons_init(on_button) == ESP_OK) {
    transport_write_literal("BUTTONS READY\n");
  } else {
    transport_write_literal("BUTTONS ERROR\n");
  }

  last_message_tick = xTaskGetTickCount();

  char line[LINE_BUFFER_BYTES];
  uint8_t input[IO_BUFFER_BYTES];
  size_t line_length = 0;
  bool discarding = false;

  transport_write_literal("READY vibebuddy-fw 0.1.0\n");

  while (true) {
    int received = uart_read_bytes(UART_NUM_0, input, sizeof(input), 0);
    if (received == 0) {
      // This wait is the key sampling period: 100 ms would miss quick taps.
      received =
          usb_serial_jtag_read_bytes(input, sizeof(input), pdMS_TO_TICKS(20));
    }

    for (int index = 0; index < received; index++) {
      char byte = (char)input[index];

      if (byte == '\n') {
        if (discarding) {
          transport_write_literal("ERROR input_too_large\n");
        } else {
          if (line_length > 0 && line[line_length - 1] == '\r') {
            line_length--;
          }
          if (line_length > MAX_LINE_BYTES) {
            transport_write_literal("ERROR input_too_large\n");
          } else {
            line[line_length] = '\0';
            last_message_tick = xTaskGetTickCount();
            set_link_lost(false);
            handle_line(line, line_length);
          }
        }
        line_length = 0;
        discarding = false;
        continue;
      }

      if (!discarding) {
        if (line_length == LINE_BUFFER_BYTES - 1) {
          discarding = true;
        } else {
          line[line_length++] = byte;
        }
      }
    }

    if ((int32_t)(xTaskGetTickCount() - last_message_tick) >=
        (int32_t)pdMS_TO_TICKS(LINK_TIMEOUT_MS)) {
      set_link_lost(true);
    }

    if (ready_scheduled &&
        (int32_t)(xTaskGetTickCount() - ready_deadline) >= 0) {
      ready_scheduled = false;
      if (agent_display_show(AGENT_DISPLAY_IDLE, NULL) != ESP_OK) {
        transport_write_literal("DISPLAY ERROR\n");
      }
    }
    agent_buttons_tick();
    handle_pomodoro_transition(agent_pomodoro_tick(clock_ms()));
    tend_leisure();
    agent_display_tick();
  }
}
