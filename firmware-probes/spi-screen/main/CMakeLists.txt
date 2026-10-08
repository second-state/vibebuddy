idf_component_register(SRCS "screen_probe.c" "audio_probe.c"
    INCLUDE_DIRS "."
    PRIV_REQUIRES esp_lcd esp_driver_spi esp_driver_gpio esp_driver_i2s)

# 复用原固件语音源文件，不复制、不改变原板的资产。
target_add_binary_data(${COMPONENT_LIB}
    "${CMAKE_CURRENT_LIST_DIR}/../../../firmware/main/assets/done.pcm"
    BINARY RENAME_TO probe_done)
