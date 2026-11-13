# Replaces ESP-IDF's bootloader main component (bootloader_start.c), as in ESP-IDF's
# examples/custom_bootloader/bootloader_override.
idf_component_register(SRCS "bootloader_start.c"
                    REQUIRES bootloader bootloader_support esp_hal_gpio spi_flash)

idf_build_get_property(scripts BOOTLOADER_LINKER_SCRIPT)
target_linker_script(${COMPONENT_LIB} INTERFACE "${scripts}")
