idf_component_register(
    SRCS "agent_beacon_fw.c" "agent_audio.c" "agent_display.c"
    PRIV_REQUIRES json esp_driver_gpio esp_driver_i2c esp_driver_i2s esp_driver_usb_serial_jtag esp_codec_dev esp_lcd
    INCLUDE_DIRS "."
    EMBED_FILES "assets/input_required.pcm" "assets/done.pcm" "assets/failed.pcm"
)
