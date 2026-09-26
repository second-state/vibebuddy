//! ST7789 的初始化与刷屏命令。
//!
//! C 固件经 esp_lcd 驱动它：`esp_lcd_new_panel_st7789` → reset → init →
//! 反色 → 再补设 MADCTL/COLMOD → swap_xy → mirror → 开显示。这里把那一串
//! 实际发出去的命令按顺序列成表，设备层照表发送。

/// 一条命令：命令字节、参数、发完之后要等的毫秒数。
pub struct Command {
    pub command: u8,
    pub parameters: &'static [u8],
    pub delay_ms: u32,
}

const fn command(command: u8, parameters: &'static [u8], delay_ms: u32) -> Command {
    Command { command, parameters, delay_ms }
}

pub const INIT_SEQUENCE: &[Command] = &[
    // 没接复位脚，软复位；规格要求至少等 5 ms，esp_lcd 等 20 ms。
    command(0x01, &[], 20),
    // 上电后在睡眠里，先出睡眠。
    command(0x11, &[], 100),
    // esp_lcd 的 init：MADCTL（RGB 顺序）、COLMOD 16 bit、RAMCTRL 大端。
    command(0x36, &[0x00], 0),
    command(0x3A, &[0x55], 0),
    command(0xB0, &[0x00, 0xF0], 0),
    // 反色：这块屏要反色才是正常颜色。
    command(0x21, &[], 0),
    // C 固件自己又补发的两条。
    command(0x36, &[0x00], 0),
    command(0x3A, &[0x65], 0),
    // 横屏：swap_xy 置 MV，再 mirror x 置 MX。
    command(0x36, &[0x20], 0),
    command(0x36, &[0x60], 0),
    // 开显示。
    command(0x29, &[], 0),
];

/// 刷整屏之前设定的窗口：列 0–319、行 0–239。之后发 RAMWR（0x2C）加像素。
pub const WINDOW: [Command; 2] = [
    command(0x2A, &[0x00, 0x00, 0x01, 0x3F], 0),
    command(0x2B, &[0x00, 0x00, 0x00, 0xEF], 0),
];

pub const RAMWR: u8 = 0x2C;
