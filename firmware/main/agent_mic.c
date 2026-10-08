#include "agent_mic.h"

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include "driver/gpio.h"
#include "driver/i2s_std.h"
#include "mbedtls/base64.h"

enum { SAMPLE_RATE = 16000, SAMPLES = SAMPLE_RATE * 2, BLOCK_FRAMES = 128 };

void agent_mic_capture(void (*write_line)(const char *)) {
  i2s_chan_handle_t rx = NULL;
  bool enabled = false;
  int16_t *pcm = malloc(SAMPLES * sizeof(*pcm));
  esp_err_t error = pcm ? ESP_OK : ESP_ERR_NO_MEM;
  if (error != ESP_OK) goto finish;

  // 与扬声器 I2S0 分开，保留 G5/G6/G7 的原有播放线路。
  i2s_chan_config_t channel = I2S_CHANNEL_DEFAULT_CONFIG(I2S_NUM_1, I2S_ROLE_MASTER);
  error = i2s_new_channel(&channel, NULL, &rx);
  if (error != ESP_OK) goto finish;
  i2s_std_config_t config = {
      .clk_cfg = I2S_STD_CLK_DEFAULT_CONFIG(SAMPLE_RATE),
      // 左右槽各 32 位，每帧 64 个时钟；L/R 接地，只取左槽高 16 位。
      .slot_cfg = I2S_STD_PHILIPS_SLOT_DEFAULT_CONFIG(I2S_DATA_BIT_WIDTH_32BIT, I2S_SLOT_MODE_STEREO),
      .gpio_cfg = {
          .mclk = I2S_GPIO_UNUSED, .bclk = GPIO_NUM_15, .ws = GPIO_NUM_16,
          .dout = I2S_GPIO_UNUSED, .din = GPIO_NUM_17,
      },
  };
  error = i2s_channel_init_std_mode(rx, &config);
  if (error != ESP_OK) goto finish;
  // 未驱动的声道为高阻，避免悬空引脚产生看似有效的噪声。
  error = gpio_set_pull_mode(GPIO_NUM_17, GPIO_PULLDOWN_ONLY);
  if (error != ESP_OK) goto finish;
  error = i2s_channel_enable(rx);
  if (error != ESP_OK) goto finish;
  enabled = true;
  write_line("MIC RECORDING 2");
  int32_t frames[BLOCK_FRAMES * 2];
  // 丢弃最初 128 ms，避开麦克风上电/时钟启动阶段。
  for (unsigned block = 0; block < 16 + SAMPLES / BLOCK_FRAMES; ++block) {
    size_t received = 0;
    error = i2s_channel_read(rx, frames, sizeof(frames), &received, 1000);
    if (error != ESP_OK) goto finish;
    if (received != sizeof(frames)) { error = ESP_ERR_INVALID_SIZE; goto finish; }
    if (block < 16) continue;
    for (unsigned i = 0; i < BLOCK_FRAMES; ++i) {
      pcm[(block - 16) * BLOCK_FRAMES + i] = (int16_t)(frames[i * 2] >> 16);
    }
  }
  error = i2s_channel_disable(rx);
  if (error != ESP_OK) goto finish;
  enabled = false;
  write_line("MIC BEGIN 16000 32000");
  for (size_t offset = 0; offset < SAMPLES * sizeof(*pcm); offset += 192) {
    unsigned char encoded[257];
    char line[288];
    size_t length = SAMPLES * sizeof(*pcm) - offset;
    if (length > 192) length = 192;
    size_t used;
    if (mbedtls_base64_encode(encoded, sizeof(encoded), &used,
                             (const unsigned char *)pcm + offset, length) != 0) {
      error = ESP_FAIL;
      goto finish;
    }
    encoded[used] = '\0';
    snprintf(line, sizeof(line), "MIC DATA %u %s", (unsigned)offset, encoded);
    write_line(line);
  }
  write_line("MIC END");

finish:
  if (enabled) (void)i2s_channel_disable(rx);
  if (rx) (void)i2s_del_channel(rx);
  free(pcm);
  if (error != ESP_OK) {
    char line[80];
    snprintf(line, sizeof(line), "MIC ERROR %s", esp_err_to_name(error));
    write_line(line);
  }
}
