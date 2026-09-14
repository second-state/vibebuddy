#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>

#include "cJSON.h"
#include "driver/usb_serial_jtag.h"
#include "esp_check.h"
#include "freertos/FreeRTOS.h"

#define MAX_LINE_BYTES 1024
#define LINE_BUFFER_BYTES (MAX_LINE_BYTES + 2)
#define USB_BUFFER_BYTES 256

static void usb_write_all(const char *data, size_t length) {
  while (length > 0) {
    int written = usb_serial_jtag_write_bytes(data, length, portMAX_DELAY);
    if (written <= 0) {
      continue;
    }
    data += written;
    length -= (size_t)written;
  }
}

static void usb_write_literal(const char *text) {
  usb_write_all(text, strlen(text));
}

static void usb_write_value_line(const char *label, const char *value) {
  usb_write_literal(label);
  while (*value != '\0') {
    char output = (*value == '\r' || *value == '\n') ? ' ' : *value;
    usb_write_all(&output, 1);
    value++;
  }
  usb_write_literal("\n");
}

static void handle_line(char *line, size_t length) {
  if (length == 0) {
    return;
  }

  cJSON *message = cJSON_ParseWithLength(line, length);
  if (message == NULL) {
    usb_write_literal("ERROR invalid_json\n");
    return;
  }

  const cJSON *version = cJSON_GetObjectItemCaseSensitive(message, "version");
  const cJSON *event = cJSON_GetObjectItemCaseSensitive(message, "event");
  const cJSON *title = cJSON_GetObjectItemCaseSensitive(message, "title");

  if (!cJSON_IsObject(message) || !cJSON_IsNumber(version) ||
      !cJSON_IsString(event) || event->valuestring[0] == '\0' ||
      (title != NULL && !cJSON_IsString(title))) {
    usb_write_literal("ERROR invalid_message\n");
    cJSON_Delete(message);
    return;
  }

  if (version->valuedouble != 1.0) {
    usb_write_literal("ERROR unsupported_version\n");
    cJSON_Delete(message);
    return;
  }

  usb_write_value_line("EVENT ", event->valuestring);
  if (title != NULL) {
    usb_write_value_line("TITLE ", title->valuestring);
  }

  cJSON_Delete(message);
}

void app_main(void) {
  usb_serial_jtag_driver_config_t usb_config = {
      .rx_buffer_size = LINE_BUFFER_BYTES,
      .tx_buffer_size = MAX_LINE_BYTES + 1,
  };
  ESP_ERROR_CHECK(usb_serial_jtag_driver_install(&usb_config));

  char line[LINE_BUFFER_BYTES];
  uint8_t input[USB_BUFFER_BYTES];
  size_t line_length = 0;
  bool discarding = false;

  usb_write_literal("READY agent-beacon-fw 0.1.0\n");

  while (true) {
    int received =
        usb_serial_jtag_read_bytes(input, sizeof(input), portMAX_DELAY);

    for (int index = 0; index < received; index++) {
      char byte = (char)input[index];

      if (byte == '\n') {
        if (discarding) {
          usb_write_literal("ERROR input_too_large\n");
        } else {
          if (line_length > 0 && line[line_length - 1] == '\r') {
            line_length--;
          }
          if (line_length > MAX_LINE_BYTES) {
            usb_write_literal("ERROR input_too_large\n");
          } else {
            line[line_length] = '\0';
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
  }
}
