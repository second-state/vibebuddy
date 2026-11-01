idf_component_register(SRCS "main.c" "panel.c" "font.c" "status.c" "buttons.c" "wifi_link.c" "buzzer.c" "microphone.c" "audio.c" INCLUDE_DIRS "." REQUIRES driver esp_timer esp_system json esp_driver_usb_serial_jtag vfs nvs_flash esp_wifi esp_event esp_netif lwip mdns bootloader_support)

set(RUST_TARGET_DIR "${CMAKE_BINARY_DIR}/rust")
set(RUST_LIB "${RUST_TARGET_DIR}/riscv32imc-unknown-none-elf/release/libvibebuddy_xggf_link.a")
add_custom_target(xggf_rust_link
    COMMAND ${CMAKE_COMMAND} -E env CARGO_TARGET_DIR=${RUST_TARGET_DIR}
        cargo build --offline --locked --release --target riscv32imc-unknown-none-elf
        --manifest-path ${CMAKE_CURRENT_LIST_DIR}/../link/Cargo.toml
    BYPRODUCTS ${RUST_LIB}
    VERBATIM)
add_dependencies(${COMPONENT_LIB} xggf_rust_link)
target_link_libraries(${COMPONENT_LIB} PUBLIC ${RUST_LIB})
