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

static bool ready_scheduled;
static TickType_t ready_deadline;
/// 超过这个时间没有收到任何消息，就认为与 Mac 端失联。
#define LINK_TIMEOUT_MS 15000
static TickType_t last_message_tick;
static bool link_lost;
/// 上一次报给 Mac 端的档位、关灯状态与小时数，只在变化时各报一行。
static agent_leisure_tier_t reported_tier = AGENT_LEISURE_ALERT;
static bool reported_lights_out;
static int reported_hour = -1;

// 两条输出都只是诊断通道，谁都不许拖住主循环。接 BOX 的 UART 桥时，USB
// Serial/JTAG 那头没有主机取数据，tx ring buffer 填满后任何等待都是永久的：
// 主任务停摆，画面定格，失联检测也一起死掉，而 Mac 端的心跳照样写得进串口，
// 两边都看不出设备已经没了。写不进去就丢掉这一段。
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

/// 当日记录变了就存一次，并报一行给 Mac 端。一天只有几次，NVS 不在乎。
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

/// 把番茄钟推到前面来；已经在前面就只重绘。
static void show_pomodoro(void) {
  if (agent_display_mode() == AGENT_MODE_POMODORO) {
    agent_display_refresh();
  } else {
    set_mode(AGENT_MODE_POMODORO);
  }
}

/// 三个键各管一件事，与模式无关：K0 是番茄钟的键，K1 在值班与番茄钟之间
/// 切换（长按去休闲），K2 交给 Mac 端去打开来源。休闲模式里任何键先把
/// 小灯灵叫回值班，再执行本职：休闲没有遮住任何需要先看一眼的东西。
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
    // 开始一个阶段时把番茄钟推到前面：圆环开始走就是反馈。
    show_pomodoro();
    return;
  }
  // 暂停与继续不换场景，小灯灵场景右上角的徽章会跟着闪。
  report_pomodoro(before.run == AGENT_POMODORO_PAUSED ? "RESUMED" : "PAUSED");
  agent_display_refresh();
}

/// 阶段结束是只消费一次的边沿：播一次语音，并把番茄钟推到前面来——
/// 这正是用户该看一眼的时刻。下一阶段停在待开始，等用户按 K0。
static void handle_pomodoro_transition(agent_pomodoro_transition_t transition) {
  if (transition == AGENT_POMODORO_NOTHING) {
    return;
  }
  bool focus_ended = transition == AGENT_POMODORO_FOCUS_ENDED;
  report_pomodoro(focus_ended ? "FOCUS END" : "BREAK END");
  if (focus_ended) {
    save_tally();
  }
  if (agent_audio_play(focus_ended ? AGENT_AUDIO_FOCUS_DONE
                                   : AGENT_AUDIO_BREAK_DONE) != ESP_OK) {
    transport_write_literal("AUDIO ERROR\n");
  }
  show_pomodoro();
}

/// 无聊度按状态累计，不按消息：Agent 有活、番茄钟在走、链路断了，都不算
/// 空闲。值班空闲够久就去休闲；有事立刻回来。番茄钟模式里，待开始的一屏
/// 静止的 25:00 和值班空闲一样无聊，五分钟就走；暂停的是用户有意停在那里
/// 的，放了半小时才当人走了。都是先回值班，接着自然会去休闲。
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
  // 档位按差异汇报，不按“本次有没有变化”：唤醒路径会先在别处推进导演。
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

/// 当日战绩随每条状态事件下发，空闲屏用它轮播。
///
/// 不能只在 `agent.idle` 上取：`task.done` 之后设备是自己回到空闲的，
/// 那一刻正是用户会看的一眼，缓存的战绩必须已经包含刚完成的这一件。
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

  // “7 DONE” 这一行的数字：休闲时它决定小灯灵是累了还是无聊。
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
  bool play_prompt = false;
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
    play_prompt = true;
    state_label = "INPUT REQUIRED";
    ready_scheduled = false;
  } else if (strcmp(event, "task.done") == 0) {
    state = AGENT_DISPLAY_DONE;
    prompt = AGENT_AUDIO_DONE;
    prompt_label = "DONE";
    play_prompt = true;
    state_label = "DONE";
    ready_scheduled = true;
    ready_deadline = xTaskGetTickCount() + pdMS_TO_TICKS(5000);
  } else if (strcmp(event, "task.error") == 0 ||
             strcmp(event, "agent.blocked") == 0) {
    state = AGENT_DISPLAY_FAILED;
    prompt = AGENT_AUDIO_FAILED;
    prompt_label = "FAILED";
    play_prompt = true;
    state_label = "FAILED";
    ready_scheduled = false;
  } else {
    return;
  }

  const cJSON *suppress_audio =
      cJSON_GetObjectItemCaseSensitive(message, "suppress_audio");
  if (cJSON_IsTrue(suppress_audio)) {
    play_prompt = false;
    prompt_label = NULL;
  }

  const cJSON *announcement =
      cJSON_GetObjectItemCaseSensitive(message, "announcement");
  if (cJSON_IsString(announcement)) {
    if (strcmp(announcement->valuestring, "done") == 0) {
      prompt = AGENT_AUDIO_DONE;
      prompt_label = "DONE";
      play_prompt = true;
    } else if (strcmp(announcement->valuestring, "failed") == 0) {
      prompt = AGENT_AUDIO_FAILED;
      prompt_label = "FAILED";
      play_prompt = true;
    }
  }

  if (agent_display_show_tasks(state, title, tasks, task_count) != ESP_OK) {
    transport_write_literal("DISPLAY ERROR\n");
  } else {
    transport_write_value_line("DISPLAY STATE ", state_label);
  }
  if (play_prompt) {
    if (agent_audio_play(prompt) != ESP_OK) {
      transport_write_literal("AUDIO ERROR\n");
    } else {
      transport_write_value_line("AUDIO QUEUED ", prompt_label);
    }
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

  // 心跳只用于证明链路存活，不显示也不回显；每 5 秒一次的诊断行会淹没日志。
  // 它顺带捎来 Mac 端的构建标识：设备可能随时重启，一次性的握手会丢。
  if (strcmp(event->valuestring, "device.heartbeat") == 0) {
    const cJSON *build = cJSON_GetObjectItemCaseSensitive(message, "build");
    if (cJSON_IsString(build)) {
      agent_display_set_daemon_build(build->valuestring);
    }
    // 本地小时数也随心跳来：设备没有时钟，白天黑夜只能听 Mac 端的。
    const cJSON *hour = cJSON_GetObjectItemCaseSensitive(message, "hour");
    if (cJSON_IsNumber(hour) && (int)hour->valuedouble != reported_hour) {
      reported_hour = (int)hour->valuedouble;
      agent_leisure_set_hour(reported_hour);
      char text[24];
      snprintf(text, sizeof(text), "CLOCK HOUR %d\n", reported_hour % 100);
      transport_write_literal(text);
    }
    // 本地日期也随心跳来：番茄钟的当日记录按它清零。
    const cJSON *day = cJSON_GetObjectItemCaseSensitive(message, "day");
    if (cJSON_IsNumber(day) && day->valuedouble > 0 &&
        agent_pomodoro_set_day((uint32_t)day->valuedouble)) {
      save_tally();
      agent_display_refresh();
    }
    cJSON_Delete(message);
    return;
  }

  // 截图是调试动作，不算 Agent 的动静，也不叫醒休闲。
  if (strcmp(event->valuestring, "device.screenshot") == 0) {
    agent_display_dump(write_shot_line);
    cJSON_Delete(message);
    return;
  }

  // Agent 一有动静，小灯灵立刻回来值班。
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

/// 版本取 esp_app_desc 里的 git 描述；时刻不取它的 __TIME__，那只在
/// esp_app_desc.c 被重编时才更新，增量构建后会停在上一次全量构建。
/// AGENT_BUILD_STAMP 由 main/CMakeLists.txt 每次构建重新生成，
/// 写法与 Mac 端一致，两行要逐字比对。
static void describe_firmware_build(char *out, size_t size) {
  const esp_app_desc_t *desc = esp_app_get_description();
  snprintf(out, size, "%.24s %.16s", desc->version, AGENT_BUILD_STAMP);
}

void app_main(void) {
  ESP_ERROR_CHECK(
      uart_driver_install(UART_NUM_0, LINE_BUFFER_BYTES, 0, 0, NULL, 0));

  usb_serial_jtag_driver_config_t usb_config = {
      .rx_buffer_size = LINE_BUFFER_BYTES,
      .tx_buffer_size = MAX_LINE_BYTES + 1,
  };
  ESP_ERROR_CHECK(usb_serial_jtag_driver_install(&usb_config));

  char firmware_build[48];
  describe_firmware_build(firmware_build, sizeof(firmware_build));

  agent_pomodoro_init();
  agent_leisure_init(esp_random(), clock_ms());
  // 昨天的记录也先恢复：换不换日要等心跳带来日期才知道。
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
    agent_display_set_firmware_build(firmware_build);
    transport_write_value_line("DISPLAY READY BUILD ", firmware_build);
  } else {
    transport_write_literal("DISPLAY ERROR\n");
  }
  if (agent_audio_init() == ESP_OK) {
    transport_write_literal("AUDIO READY\n");
    transport_write_value_line("AUDIO CODEC ", agent_audio_status());
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

  transport_write_literal("READY agent-beacon-fw 0.1.0\n");

  while (true) {
    int received = uart_read_bytes(UART_NUM_0, input, sizeof(input), 0);
    if (received == 0) {
      // 这里的等待就是按键的采样周期：100 ms 会漏掉短促的轻点。
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
