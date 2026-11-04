idf_component_register(SRCS "main.c" "clock.c" "rtc.c" "volume.c" "panel.c" "font.c" "status.c" "buttons.c" "button_actions.c" "settings.c" "wifi_link.c" "buzzer.c" "microphone.c" "audio.c" INCLUDE_DIRS "." REQUIRES driver esp_timer esp_system json esp_driver_usb_serial_jtag vfs nvs_flash esp_wifi esp_event esp_netif lwip mdns bootloader_support)

option(XGGF_BUTTON_AUTOPILOT "Enable USB-injected button state-machine tests" OFF)
if(XGGF_BUTTON_AUTOPILOT)
    target_compile_definitions(${COMPONENT_LIB} PRIVATE XGGF_BUTTON_AUTOPILOT=1)
endif()

set(XGGF_POMODORO_TIME_SCALE "1" CACHE STRING "Pomodoro acceptance time divisor (1 or 60)")
set(POMODORO_FEATURE_ARGS)
if(XGGF_POMODORO_TIME_SCALE STREQUAL "60")
    if(NOT XGGF_BUTTON_AUTOPILOT)
        message(FATAL_ERROR "Accelerated Pomodoro requires the autopilot acceptance build")
    endif()
    list(APPEND POMODORO_FEATURE_ARGS --features fast-clock)
elseif(NOT XGGF_POMODORO_TIME_SCALE STREQUAL "1")
    message(FATAL_ERROR "XGGF_POMODORO_TIME_SCALE must be 1 or 60")
endif()
set(RUST_TARGET_DIR "${CMAKE_BINARY_DIR}/rust")
set(RUST_LIB "${RUST_TARGET_DIR}/riscv32imc-unknown-none-elf/release/libvibebuddy_xggf_link.a")
add_custom_target(xggf_rust_link
    COMMAND ${CMAKE_COMMAND} -E env CARGO_TARGET_DIR=${RUST_TARGET_DIR}
        cargo build --offline --locked --release --target riscv32imc-unknown-none-elf
        --manifest-path ${CMAKE_CURRENT_LIST_DIR}/../link/Cargo.toml ${POMODORO_FEATURE_ARGS}
    BYPRODUCTS ${RUST_LIB}
    VERBATIM)
add_dependencies(${COMPONENT_LIB} xggf_rust_link)
target_link_libraries(${COMPONENT_LIB} PUBLIC ${RUST_LIB})

option(XGGF_RTC_WRITE "Enable experimental stock BCD RTC writes" OFF)
if(XGGF_RTC_WRITE)
 target_compile_definitions(${COMPONENT_LIB} PRIVATE XGGF_RTC_WRITE=1)
endif()
