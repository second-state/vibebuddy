#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "agent_audio.h"
#include "agent_build_stamp.h"
#include "agent_buttons.h"
#include "agent_display.h"
#include "agent_pomodoro.h"
#include "cJSON.h"
#include "driver/uart.h"
#include "esp_app_desc.h"
#include "driver/usb_serial_jtag.h"
#include "esp_check.h"
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

static const char *scene_name(agent_scene_t scene) {
  return scene == AGENT_SCENE_POMODORO ? "POMODORO" : "PET";
}

static void report_pomodoro(const char *what) {
  transport_write_value_line("POMODORO ", what);
}

/// 把番茄钟推到前面来；已经在前面就只重绘。
static void show_pomodoro(void) {
  if (agent_display_scene() == AGENT_SCENE_POMODORO) {
    agent_display_refresh();
  } else {
    agent_display_set_scene(AGENT_SCENE_POMODORO);
  }
}

/// 三个键各管一件事，与场景无关：K0 是番茄钟的键，K1 换场景，K2 交给
/// Mac 端去打开来源。
static void on_button(agent_button_event_t event) {
  if (event == AGENT_BUTTON_K2_SHORT) {
    transport_write_literal(
        "{\"version\":1,\"event\":\"button\",\"button\":\"K2\",\"action\":\"press\"}\n");
    return;
  }
  if (event == AGENT_BUTTON_K1_SHORT) {
    agent_scene_t next = agent_display_scene() == AGENT_SCENE_PET
                             ? AGENT_SCENE_POMODORO
                             : AGENT_SCENE_PET;
    agent_display_set_scene(next);
    transport_write_value_line("SCENE ", scene_name(next));
    return;
  }

  agent_pomodoro_view_t before;
  agent_pomodoro_view(clock_ms(), &before);
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
  agent_pomodoro_toggle(clock_ms());
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
  if (agent_audio_play(focus_ended ? AGENT_AUDIO_FOCUS_DONE
                                   : AGENT_AUDIO_BREAK_DONE) != ESP_OK) {
    transport_write_literal("AUDIO ERROR\n");
  }
  show_pomodoro();
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
    tasks[count].title = title->valuestring;
    tasks[count].state = task_state(status->valuestring);
    tasks[count].elapsed_s = cJSON_IsNumber(elapsed) ? (int)elapsed->valuedouble : 0;
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
    cJSON_Delete(message);
    return;
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
            agent_display_set_link_lost(false);
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
      agent_display_set_link_lost(true);
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
    agent_display_tick();
  }
}
