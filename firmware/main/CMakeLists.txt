idf_component_register(
    SRCS "agent_beacon_fw.c" "agent_audio.c" "agent_buttons.c" "agent_display.c" "agent_pomodoro.c"
    PRIV_REQUIRES json esp_app_format esp_driver_gpio esp_driver_i2c esp_driver_i2s esp_driver_uart esp_driver_usb_serial_jtag esp_codec_dev esp_lcd
    INCLUDE_DIRS "."
    EMBED_FILES "assets/input_required.pcm" "assets/done.pcm" "assets/failed.pcm" "assets/focus_done.pcm" "assets/break_done.pcm"
)

# 页脚里的编译时刻由这里每次构建重新生成。esp_app_desc.c 里的 __TIME__ 只在
# 它自己被重编时更新，增量构建之后会停在上一次全量构建的时刻。第二个 OUTPUT
# 永远不会被生成，Ninja 因此每次都认为这条命令过期。
set(build_stamp_header "${CMAKE_CURRENT_BINARY_DIR}/agent_build_stamp.h")
add_custom_command(
    OUTPUT "${build_stamp_header}" "${CMAKE_CURRENT_BINARY_DIR}/agent_build_stamp.never"
    COMMAND ${CMAKE_COMMAND} -DOUTPUT_FILE=${build_stamp_header}
            -P "${CMAKE_CURRENT_LIST_DIR}/build_stamp.cmake"
    COMMENT "记录本次构建时刻"
    VERBATIM
)
add_custom_target(agent_build_stamp DEPENDS "${build_stamp_header}")
add_dependencies(${COMPONENT_LIB} agent_build_stamp)
set_property(SOURCE agent_beacon_fw.c APPEND PROPERTY OBJECT_DEPENDS "${build_stamp_header}")
target_include_directories(${COMPONENT_LIB} PRIVATE "${CMAKE_CURRENT_BINARY_DIR}")
