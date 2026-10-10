# Builds only the ESP-IDF second-stage bootloader the Rust firmware boots through (ADR-0013); the app here is a stub.
# tools/build-bootloader.sh builds it and copies the result to bootloader.bin next to this file.
cmake_minimum_required(VERSION 3.16)
include($ENV{IDF_PATH}/tools/cmake/project.cmake)
project(vibebuddy-bootloader)
