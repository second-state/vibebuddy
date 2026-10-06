idf_component_register(
    SRCS "vibebuddy_fw.c" "agent_audio.c" "agent_buttons.c" "agent_display.c" "agent_leisure.c" "agent_pomodoro.c" "agent_tally.c" "agent_voice_pack.c" "agent_voices.c"
    PRIV_REQUIRES json esp_app_format esp_driver_gpio esp_driver_i2c esp_driver_i2s esp_driver_uart esp_driver_usb_serial_jtag esp_codec_dev esp_hw_support esp_lcd esp_partition mbedtls nvs_flash spi_flash
    INCLUDE_DIRS "."
    EMBED_FILES "assets/input_required.pcm" "assets/done.pcm" "assets/failed.pcm" "assets/focus_done.pcm" "assets/break_done.pcm"
)

# The build time in the build stamp is regenerated here on every build. __TIME__ in
# esp_app_desc.c only updates when that file is recompiled, so after incremental
# builds it sticks at the last full build. The second OUTPUT is never produced,
# so Ninja always considers this command out of date.
set(build_stamp_header "${CMAKE_CURRENT_BINARY_DIR}/agent_build_stamp.h")
add_custom_command(
    OUTPUT "${build_stamp_header}" "${CMAKE_CURRENT_BINARY_DIR}/agent_build_stamp.never"
    COMMAND ${CMAKE_COMMAND} -DOUTPUT_FILE=${build_stamp_header}
            -P "${CMAKE_CURRENT_LIST_DIR}/build_stamp.cmake"
    COMMENT "Recording the build timestamp"
    VERBATIM
)
# For on-device acceptance: idf.py -B firmware/build-fast -DTIME_SCALE=60 build
# squeezes leisure's five minutes into five seconds and the pomodoro's 25 minutes
# into 25 seconds, so the whole chain runs in a few minutes.
# Release firmware doesn't use it.
if(DEFINED TIME_SCALE)
    target_compile_definitions(${COMPONENT_LIB} PRIVATE
        AGENT_TIME_SCALE=${TIME_SCALE}u)
endif()

add_custom_target(agent_build_stamp DEPENDS "${build_stamp_header}")
add_dependencies(${COMPONENT_LIB} agent_build_stamp)
set_property(SOURCE vibebuddy_fw.c APPEND PROPERTY OBJECT_DEPENDS "${build_stamp_header}")
target_include_directories(${COMPONENT_LIB} PRIVATE "${CMAKE_CURRENT_BINARY_DIR}")
