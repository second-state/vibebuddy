/*
 * Vibe Buddy's second-stage bootloader: ESP-IDF's bootloader_start.c (v6.0.1), with one change to how it picks the
 * app. A box can carry Muse next to Vibe Buddy (docs/muse.md). The boot_sel partition says which of the two boots,
 * and holding K2 at reset toggles it. Neither app knows about the other: only this file does. The before- and
 * after-init hooks are left out: their header lives in the main component this one replaces, and nothing uses them.
 *
 * Based on ESP-IDF components/bootloader/subproject/main/bootloader_start.c,
 * SPDX-FileCopyrightText: 2015-2024 Espressif Systems (Shanghai) CO LTD
 * SPDX-License-Identifier: Apache-2.0
 */
#include <stdbool.h>
#include <string.h>
#include "sdkconfig.h"
#include "esp_log.h"
#include "esp_rom_sys.h"
#include "esp_rom_gpio.h"
#include "esp_flash_partitions.h"
#include "esp_image_format.h"
#include "bootloader_init.h"
#include "bootloader_utility.h"
#include "bootloader_common.h"
#include "bootloader_flash_priv.h"
#include "hal/gpio_ll.h"

ESP_LOG_ATTR_TAG(TAG, "boot");

static int select_partition_number(bootloader_state_t *bs);
static int selected_boot_partition(const bootloader_state_t *bs);

void __attribute__((noreturn)) call_start_cpu0(void)
{
    if (bootloader_init() != ESP_OK) {
        bootloader_reset();
    }
    bootloader_state_t bs = {0};
    int boot_index = select_partition_number(&bs);
    if (boot_index == INVALID_INDEX) {
        bootloader_reset();
    }
    bootloader_utility_load_boot_image(&bs, boot_index);
}

static int select_partition_number(bootloader_state_t *bs)
{
    if (!bootloader_utility_load_partition_table(bs)) {
        ESP_LOGE(TAG, "load partition table error!");
        return INVALID_INDEX;
    }
    return selected_boot_partition(bs);
}

/* ---- K2, on the XL9555 expander, read over a bit-banged I2C ---- */

#define I2C_SDA 48
#define I2C_SCL 45
#define XL9555_ADDRESS 0x20
#define XL9555_INPUT_PORT0 0x00
#define XL9555_K2 (1u << 3) /* active low */
#define HALF_BIT_US 5       /* about 100 kHz */

/* Open drain by hand: a line is high when released (the board's pull-ups) and low when driven. */
static void line_release(int pin)
{
    gpio_ll_output_disable(&GPIO, pin);
}

static void line_low(int pin)
{
    gpio_ll_set_level(&GPIO, pin, 0);
    gpio_ll_output_enable(&GPIO, pin);
}

static int line_read(int pin)
{
    return gpio_ll_get_level(&GPIO, pin);
}

static void i2c_pins_init(void)
{
    const int pins[] = { I2C_SDA, I2C_SCL };
    for (int i = 0; i < 2; i++) {
        int pin = pins[i];
        esp_rom_gpio_pad_select_gpio(pin);
        gpio_ll_input_enable(&GPIO, pin);
        gpio_ll_pullup_en(&GPIO, pin);
        line_release(pin);
    }
}

static void i2c_start(void)
{
    line_release(I2C_SDA);
    line_release(I2C_SCL);
    esp_rom_delay_us(HALF_BIT_US);
    line_low(I2C_SDA);
    esp_rom_delay_us(HALF_BIT_US);
    line_low(I2C_SCL);
}

static void i2c_stop(void)
{
    line_low(I2C_SDA);
    esp_rom_delay_us(HALF_BIT_US);
    line_release(I2C_SCL);
    esp_rom_delay_us(HALF_BIT_US);
    line_release(I2C_SDA);
    esp_rom_delay_us(HALF_BIT_US);
}

/* Clocks one bit out (or in, with the line released) and returns what SDA was while SCL was high. */
static int i2c_bit(int bit)
{
    if (bit) {
        line_release(I2C_SDA);
    } else {
        line_low(I2C_SDA);
    }
    esp_rom_delay_us(HALF_BIT_US);
    line_release(I2C_SCL);
    esp_rom_delay_us(HALF_BIT_US);
    int seen = line_read(I2C_SDA);
    line_low(I2C_SCL);
    return seen;
}

/* Sends a byte; true when the device acknowledged it. */
static bool i2c_write_byte(uint8_t byte)
{
    for (int i = 7; i >= 0; i--) {
        i2c_bit((byte >> i) & 1);
    }
    return i2c_bit(1) == 0;
}

static uint8_t i2c_read_byte_nack(void)
{
    uint8_t byte = 0;
    for (int i = 0; i < 8; i++) {
        byte = (byte << 1) | i2c_bit(1);
    }
    i2c_bit(1); /* NACK: the last byte */
    return byte;
}

/* Reads XL9555 input port 0. False when nothing answers, as on hardware without the expander. */
static bool xl9555_read_input(uint8_t *value)
{
    i2c_start();
    bool ok = i2c_write_byte(XL9555_ADDRESS << 1) && i2c_write_byte(XL9555_INPUT_PORT0);
    if (ok) {
        i2c_start(); /* repeated start */
        ok = i2c_write_byte((XL9555_ADDRESS << 1) | 1);
        if (ok) {
            *value = i2c_read_byte_nack();
        }
    }
    i2c_stop();
    return ok;
}

/* True when K2 stays down for half a second from reset. */
static bool k2_held(void)
{
    i2c_pins_init();
    for (int sample = 0; sample < 10; sample++) {
        uint8_t input;
        if (!xl9555_read_input(&input) || (input & XL9555_K2)) {
            return false;
        }
        esp_rom_delay_us(50 * 1000);
    }
    return true;
}

/* ---- boot_sel: which app boots ---- */

/* The first word of boot_sel. Anything else, a blank sector included, means Vibe Buddy. */
#define BOOT_MUSE 0x4553554D /* "MUSE" */

static bool find_partition(const char *label, esp_partition_pos_t *pos)
{
    const esp_partition_info_t *table = bootloader_mmap(ESP_PARTITION_TABLE_OFFSET, ESP_PARTITION_TABLE_MAX_LEN);
    if (!table) {
        return false;
    }
    bool found = false;
    for (int i = 0; i < ESP_PARTITION_TABLE_MAX_ENTRIES && table[i].magic == ESP_PARTITION_MAGIC; i++) {
        if (strncmp((const char *)table[i].label, label, sizeof(table[i].label)) == 0) {
            *pos = table[i].pos;
            found = true;
            break;
        }
    }
    bootloader_munmap(table);
    return found;
}

static void write_boot_sel(const esp_partition_pos_t *boot_sel, bool muse)
{
    if (bootloader_flash_erase_sector(boot_sel->offset / FLASH_SECTOR_SIZE) != ESP_OK) {
        ESP_LOGE(TAG, "can't erase boot_sel");
        return;
    }
    if (muse) {
        uint32_t word = BOOT_MUSE;
        if (bootloader_flash_write(boot_sel->offset, &word, sizeof(word), false) != ESP_OK) {
            ESP_LOGE(TAG, "can't write boot_sel");
        }
    }
}

/* True when Muse should boot this time. Touches the I2C pins only on a box that has a Muse image. */
static bool boot_muse(const bootloader_state_t *bs)
{
    esp_partition_pos_t boot_sel;
    uint8_t first_byte = 0xFF;
    if (bs->test.offset == 0 || !find_partition("boot_sel", &boot_sel)
        || bootloader_flash_read(bs->test.offset, &first_byte, 1, false) != ESP_OK || first_byte != ESP_IMAGE_HEADER_MAGIC) {
        return false;
    }
    uint32_t word = 0;
    bool muse = bootloader_flash_read(boot_sel.offset, &word, sizeof(word), false) == ESP_OK && word == BOOT_MUSE;
    /* Deep sleep wake keeps the choice: Muse powers off into deep sleep. */
    if (esp_rom_get_reset_reason(0) != RESET_REASON_CORE_DEEP_SLEEP && k2_held()) {
        muse = !muse;
        ESP_LOGI(TAG, "K2 held: switching to %s", muse ? "Muse" : "Vibe Buddy");
        write_boot_sel(&boot_sel, muse);
    }
    if (!muse) {
        return false;
    }
    /* bootloader_utility_load_boot_image() resets on a bad test image, and would do so on every boot. */
    esp_image_metadata_t image = {0};
    if (esp_image_verify(ESP_IMAGE_VERIFY_SILENT, &bs->test, &image) != ESP_OK) {
        ESP_LOGE(TAG, "the Muse image is damaged; booting Vibe Buddy");
        write_boot_sel(&boot_sel, false);
        return false;
    }
    return true;
}

static int selected_boot_partition(const bootloader_state_t *bs)
{
    /* Before otadata is read: reading it can move a new Vibe Buddy image on to its trial boot, which booting Muse
     * instead would waste. */
    if (boot_muse(bs)) {
        ESP_LOGI(TAG, "booting Muse");
        return TEST_APP_INDEX;
    }
    return bootloader_utility_get_selected_boot_partition(bs);
}

#if CONFIG_LIBC_NEWLIB
struct _reent *__getreent(void)
{
    return _GLOBAL_REENT;
}
#endif
