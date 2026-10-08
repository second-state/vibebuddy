#include "agent_audio.h"

#include "agent_voices.h"

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "driver/i2c_master.h"
#include "driver/i2s_std.h"
#include "esp_check.h"
#include "esp_codec_dev.h"
#include "esp_codec_dev_defaults.h"
#include "nvs.h"
#include "esp_log.h"
#include "freertos/FreeRTOS.h"
#include "freertos/queue.h"
#include "freertos/task.h"

#define AUDIO_SAMPLE_RATE 24000
#ifdef AGENT_BOARD_BREADBOARD
#define AUDIO_I2S_BCLK GPIO_NUM_5
#define AUDIO_I2S_WS GPIO_NUM_6
#define AUDIO_I2S_DOUT GPIO_NUM_7
#else
#define AUDIO_I2S_BCLK GPIO_NUM_21
#define AUDIO_I2S_WS GPIO_NUM_13
#define AUDIO_I2S_DOUT GPIO_NUM_14
#endif

#define ES8311_ADDRESS 0x18
#define XL9555_ADDRESS 0x20
#define XL9555_OUTPUT_PORT0 0x02
#define XL9555_CONFIG_PORT0 0x06
#define XL9555_SPEAKER_MASK 0x20

#define VOLUME_NAMESPACE "audio"
#define VOLUME_KEY "volume"

static const char *TAG = "agent_audio";
static i2s_chan_handle_t tx_handle;
/// 只有 ES8311 版本才有；NS4168 版本没有音量可调。
#ifndef AGENT_BOARD_BREADBOARD
static esp_codec_dev_handle_t codec_device;
#endif
static unsigned volume = AGENT_AUDIO_VOLUME_DEFAULT;
static QueueHandle_t prompt_queue;
static bool audio_ready;
static volatile bool playing;
static const char *audio_status = "NOT INITIALIZED";

#ifndef AGENT_BOARD_BREADBOARD
static esp_err_t xl9555_read(i2c_master_dev_handle_t handle, uint8_t reg,
                             uint8_t *value) {
  return i2c_master_transmit_receive(handle, &reg, 1, value, 1,
                                     pdMS_TO_TICKS(100));
}

static esp_err_t xl9555_write(i2c_master_dev_handle_t handle, uint8_t reg,
                              uint8_t value) {
  uint8_t command[] = {reg, value};
  return i2c_master_transmit(handle, command, sizeof(command),
                             pdMS_TO_TICKS(100));
}

static esp_err_t enable_speaker(i2c_master_bus_handle_t i2c_bus) {
  i2c_device_config_t device_config = {
      .dev_addr_length = I2C_ADDR_BIT_LEN_7,
      .device_address = XL9555_ADDRESS,
      .scl_speed_hz = 400000,
  };
  i2c_master_dev_handle_t handle;
  ESP_RETURN_ON_ERROR(
      i2c_master_bus_add_device(i2c_bus, &device_config, &handle), TAG,
      "添加 XL9555 音频控制失败");

  uint8_t direction;
  ESP_RETURN_ON_ERROR(xl9555_read(handle, XL9555_CONFIG_PORT0, &direction), TAG,
                      "读取 XL9555 direction 失败");
  direction &= (uint8_t)~XL9555_SPEAKER_MASK;
  ESP_RETURN_ON_ERROR(xl9555_write(handle, XL9555_CONFIG_PORT0, direction), TAG,
                      "配置扬声器使能方向失败");

  uint8_t output;
  ESP_RETURN_ON_ERROR(xl9555_read(handle, XL9555_OUTPUT_PORT0, &output), TAG,
                      "读取 XL9555 output 失败");
  output |= XL9555_SPEAKER_MASK;
  return xl9555_write(handle, XL9555_OUTPUT_PORT0, output);
}

#endif

static unsigned clamp_volume(unsigned level) {
  if (level < AGENT_AUDIO_VOLUME_MIN) {
    return AGENT_AUDIO_VOLUME_MIN;
  }
  if (level > AGENT_AUDIO_VOLUME_MAX) {
    return AGENT_AUDIO_VOLUME_MAX;
  }
  return level;
}

/// 开机读回上次的音量；没存过就是默认值。NVS 由 agent_tally_init 先初始化。
static void load_volume(void) {
  nvs_handle_t handle;
  if (nvs_open(VOLUME_NAMESPACE, NVS_READONLY, &handle) != ESP_OK) {
    return;
  }
  uint32_t stored;
  if (nvs_get_u32(handle, VOLUME_KEY, &stored) == ESP_OK) {
    volume = clamp_volume((unsigned)stored);
  }
  nvs_close(handle);
}

static esp_err_t save_volume(void) {
  nvs_handle_t handle;
  ESP_RETURN_ON_ERROR(nvs_open(VOLUME_NAMESPACE, NVS_READWRITE, &handle), TAG,
                      "打开 NVS 失败");
  esp_err_t result = nvs_set_u32(handle, VOLUME_KEY, volume);
  if (result == ESP_OK) {
    result = nvs_commit(handle);
  }
  nvs_close(handle);
  return result;
}

static esp_err_t init_i2s(void) {
  i2s_chan_config_t channel_config =
      I2S_CHANNEL_DEFAULT_CONFIG(I2S_NUM_0, I2S_ROLE_MASTER);
  channel_config.auto_clear = true;
  ESP_RETURN_ON_ERROR(i2s_new_channel(&channel_config, &tx_handle, NULL), TAG,
                      "创建 I2S TX 失败");

  i2s_std_config_t standard_config = {
      .clk_cfg = I2S_STD_CLK_DEFAULT_CONFIG(AUDIO_SAMPLE_RATE),
      .slot_cfg = I2S_STD_PHILIPS_SLOT_DEFAULT_CONFIG(I2S_DATA_BIT_WIDTH_16BIT,
                                                      I2S_SLOT_MODE_STEREO),
      .gpio_cfg =
          {
              .mclk = I2S_GPIO_UNUSED,
              .bclk = AUDIO_I2S_BCLK,
              .ws = AUDIO_I2S_WS,
              .dout = AUDIO_I2S_DOUT,
              .din = I2S_GPIO_UNUSED,
          },
  };
  standard_config.clk_cfg.mclk_multiple = I2S_MCLK_MULTIPLE_256;
  ESP_RETURN_ON_ERROR(i2s_channel_init_std_mode(tx_handle, &standard_config),
                      TAG, "初始化 I2S 标准模式失败");
  return i2s_channel_enable(tx_handle);
}

#ifndef AGENT_BOARD_BREADBOARD
static esp_err_t init_es8311(i2c_master_bus_handle_t i2c_bus) {
  audio_codec_i2c_cfg_t i2c_config = {
      .port = I2C_NUM_0,
      // esp_codec_dev 接收 8 位线地址，内部再转换成 7 位地址。
      .addr = ES8311_ADDRESS << 1,
      .bus_handle = i2c_bus,
  };
  const audio_codec_ctrl_if_t *control = audio_codec_new_i2c_ctrl(&i2c_config);
  if (control == NULL) {
    audio_status = "ES8311 CONTROL";
    return ESP_ERR_NO_MEM;
  }

  audio_codec_i2s_cfg_t i2s_config = {
      .port = I2S_NUM_0,
      .rx_handle = NULL,
      .tx_handle = tx_handle,
  };
  const audio_codec_data_if_t *data = audio_codec_new_i2s_data(&i2s_config);
  const audio_codec_gpio_if_t *gpio = audio_codec_new_gpio();
  if (data == NULL || gpio == NULL) {
    audio_status = "ES8311 DATA";
    return ESP_ERR_NO_MEM;
  }

  es8311_codec_cfg_t codec_config = {
      .ctrl_if = control,
      .gpio_if = gpio,
      .codec_mode = ESP_CODEC_DEV_WORK_MODE_BOTH,
      .master_mode = false,
      .use_mclk = false,
      .pa_pin = GPIO_NUM_NC,
      .pa_reverted = false,
      .hw_gain =
          {
              .pa_voltage = 5.0,
              .codec_dac_voltage = 3.3,
          },
      .mclk_div = 256,
  };
  const audio_codec_if_t *codec = es8311_codec_new(&codec_config);
  if (codec == NULL) {
    audio_status = "ES8311 CODEC";
    return ESP_ERR_NO_MEM;
  }

  esp_codec_dev_cfg_t device_config = {
      .dev_type = ESP_CODEC_DEV_TYPE_OUT,
      .codec_if = codec,
      .data_if = data,
  };
  esp_codec_dev_handle_t device = esp_codec_dev_new(&device_config);
  if (device == NULL) {
    audio_status = "ES8311 DEVICE";
    return ESP_ERR_NO_MEM;
  }

  esp_codec_dev_sample_info_t sample_config = {
      .bits_per_sample = 16,
      .channel = 2,
      .channel_mask = 0x03,
      .sample_rate = AUDIO_SAMPLE_RATE,
  };
  if (esp_codec_dev_open(device, &sample_config) != ESP_CODEC_DEV_OK) {
    audio_status = "ES8311 OPEN";
    return ESP_FAIL;
  }
  if (esp_codec_dev_set_out_vol(device, (int)volume) != ESP_CODEC_DEV_OK) {
    audio_status = "ES8311 VOLUME";
    return ESP_FAIL;
  }
  codec_device = device;
  return ESP_OK;
}

#endif

static void audio_task(void *argument) {
  (void)argument;
  while (true) {
    agent_audio_prompt_t prompt;
    if (xQueueReceive(prompt_queue, &prompt, portMAX_DELAY) != pdTRUE) {
      continue;
    }

    // 先标记在播再取指针：语音包写入那边靠这个顺序判断映射区还有没有人用。
    playing = true;
    __sync_synchronize();
    const uint8_t *start;
    size_t length;
    agent_voices_clip(prompt, &start, &length);
    const uint8_t *end = start + length;

    size_t bytes_written = 0;
#ifdef AGENT_BOARD_BREADBOARD
    // 当前 3V3 面包板只验证过低音量；最大档限制为原 PCM 振幅的 1/16。
    // 解码字节避免依赖语音包映射地址的对齐方式。
    int16_t samples[512];
    const unsigned level = volume;
    esp_err_t result = ESP_OK;
    while (bytes_written < length) {
      size_t chunk = length - bytes_written;
      if (chunk > sizeof(samples)) chunk = sizeof(samples);
      if (chunk % 4 != 0) { result = ESP_ERR_INVALID_SIZE; break; }
      for (size_t i = 0; i < chunk / 2; ++i) {
        const uint8_t *p = start + bytes_written + i * 2;
        int16_t sample = (int16_t)((unsigned)p[0] | ((unsigned)p[1] << 8));
        samples[i] = (int16_t)((int32_t)sample * (int)level / 1600);
      }
      size_t written = 0;
      result = i2s_channel_write(tx_handle, samples, chunk, &written, 1000);
      bytes_written += written;
      if (result != ESP_OK || written != chunk) break;
    }
#else
    esp_err_t result = i2s_channel_write(tx_handle, start, end - start,
                                         &bytes_written, portMAX_DELAY);
#endif
    playing = false;
    if (result != ESP_OK || bytes_written != (size_t)(end - start)) {
      ESP_LOGE(TAG, "语音播放失败: %s, %u/%u bytes", esp_err_to_name(result),
               (unsigned)bytes_written, (unsigned)(end - start));
    }
  }
}

esp_err_t agent_audio_init(void) {
  load_volume();
#ifdef AGENT_BOARD_BREADBOARD
  esp_err_t result = init_i2s();
  if (result != ESP_OK) {
    audio_status = "I2S";
    return result;
  }
  audio_status = "MAX98357A";
#else
  i2c_master_bus_handle_t i2c_bus;
  esp_err_t result = i2c_master_get_bus_handle(I2C_NUM_0, &i2c_bus);
  if (result != ESP_OK) {
    audio_status = "I2C BUS";
    return result;
  }
  result = init_i2s();
  if (result != ESP_OK) {
    audio_status = "I2S";
    return result;
  }

  esp_err_t probe_result =
      i2c_master_probe(i2c_bus, ES8311_ADDRESS, pdMS_TO_TICKS(100));
  if (probe_result == ESP_OK) {
    result = init_es8311(i2c_bus);
    if (result != ESP_OK) {
      return result;
    }
    audio_status = "ES8311";
    ESP_LOGI(TAG, "检测到 ES8311 音频版本");
  } else if (probe_result == ESP_ERR_NOT_FOUND) {
    audio_status = "NS4168";
    ESP_LOGI(TAG, "未检测到 ES8311，使用 NS4168 I2S 音频版本");
  } else {
    audio_status = "ES8311 PROBE";
    return probe_result;
  }

  result = enable_speaker(i2c_bus);
  if (result != ESP_OK) {
    audio_status = "SPEAKER ENABLE";
    return result;
  }
#endif
  prompt_queue = xQueueCreate(8, sizeof(agent_audio_prompt_t));
  if (prompt_queue == NULL) {
    audio_status = "AUDIO QUEUE";
    return ESP_ERR_NO_MEM;
  }
  if (xTaskCreate(audio_task, "agent_audio", 4096, NULL, 5, NULL) != pdPASS) {
    audio_status = "AUDIO TASK";
    return ESP_ERR_NO_MEM;
  }
  audio_ready = true;
  return ESP_OK;
}

esp_err_t agent_audio_play(agent_audio_prompt_t prompt) {
  if (!audio_ready) {
    return ESP_ERR_INVALID_STATE;
  }
  return xQueueSend(prompt_queue, &prompt, 0) == pdTRUE ? ESP_OK
                                                        : ESP_ERR_NO_MEM;
}

const char *agent_audio_status(void) { return audio_status; }

esp_err_t agent_audio_set_volume(unsigned level) {
#ifndef AGENT_BOARD_BREADBOARD
  if (codec_device == NULL) {
    return ESP_ERR_NOT_SUPPORTED;
  }
  level = clamp_volume(level);
  if (esp_codec_dev_set_out_vol(codec_device, (int)level) !=
      ESP_CODEC_DEV_OK) {
    return ESP_FAIL;
  }
#else
  level = clamp_volume(level);
#endif
  volume = level;
  return save_volume();
}

unsigned agent_audio_volume(void) { return volume; }

bool agent_audio_playing(void) { return playing; }
