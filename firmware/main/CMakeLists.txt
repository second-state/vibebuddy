idf_component_register(
    SRCS "agent_beacon_fw.c" "agent_display.c"
    PRIV_REQUIRES json esp_driver_gpio esp_driver_i2c esp_driver_usb_serial_jtag esp_lcd
    INCLUDE_DIRS "."
)
