idf_component_register(
    SRCS "agent_beacon_fw.c"
    PRIV_REQUIRES json esp_driver_usb_serial_jtag
    INCLUDE_DIRS "."
)
