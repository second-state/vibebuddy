//! 用 ESP32-S3 的 ROM 串口下载协议烧固件，不加载 stub。
//!
//! 不用 espflash：它的 ROM 写块固定 1 KB、串口对象是具体类型没法包装，而
//! BOX 的 CH343 桥一次只吞得下两百来字节（见 serial_transport 的分段注释；
//! 之前的烧录脚本也是把 esptool 的块改成 0x100 才走通的）。这里块 256 字节，
//! 桥接时每 128 字节按线速分段写。协议照 esptool：SLIP 封包、SYNC、READ_REG
//! 认芯片、SPI_ATTACH、FLASH_BEGIN/DATA/END、SPI_FLASH_MD5 校验、RTS 硬复位。

use std::io::{Read, Write};
use std::time::{Duration, Instant};

use serialport::SerialPort;

const BAUD: u32 = 115_200;
const BLOCK_BYTES: usize = 256;
const SLIP_END: u8 = 0xC0;
const SLIP_ESC: u8 = 0xDB;
const SLIP_ESC_END: u8 = 0xDC;
const SLIP_ESC_ESC: u8 = 0xDD;

const OP_FLASH_BEGIN: u8 = 0x02;
const OP_FLASH_DATA: u8 = 0x03;
const OP_SYNC: u8 = 0x08;
const OP_READ_REG: u8 = 0x0A;
const OP_SPI_SET_PARAMS: u8 = 0x0B;
const OP_SPI_ATTACH: u8 = 0x0D;
const OP_SPI_FLASH_MD5: u8 = 0x13;

const CHIP_MAGIC_REG: u32 = 0x4000_1000;
const ESP32S3_MAGIC: u32 = 0x9;
/// ROM loader 的应答末尾带 4 个状态字节（stub 是 2 个）。
const STATUS_BYTES: usize = 4;
const CHECKSUM_SEED: u8 = 0xEF;

#[derive(Clone, Debug)]
pub struct Segment {
    pub address: u32,
    pub data: Vec<u8>,
}

pub struct Progress<'a> {
    pub on_progress: &'a mut dyn FnMut(f32, &str),
}

/// SLIP 封包。
pub fn slip_encode(payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(payload.len() + 2);
    frame.push(SLIP_END);
    for byte in payload {
        match *byte {
            SLIP_END => frame.extend_from_slice(&[SLIP_ESC, SLIP_ESC_END]),
            SLIP_ESC => frame.extend_from_slice(&[SLIP_ESC, SLIP_ESC_ESC]),
            other => frame.push(other),
        }
    }
    frame.push(SLIP_END);
    frame
}

/// 命令包：方向 0、操作码、数据长度、校验（只有 FLASH_DATA 用）、数据。
pub fn command(op: u8, data: &[u8], checksum: u32) -> Vec<u8> {
    let mut packet = Vec::with_capacity(8 + data.len());
    packet.push(0x00);
    packet.push(op);
    packet.extend_from_slice(&(data.len() as u16).to_le_bytes());
    packet.extend_from_slice(&checksum.to_le_bytes());
    packet.extend_from_slice(data);
    packet
}

/// FLASH_DATA 的校验：所有数据字节异或，种子 0xEF。
pub fn checksum(data: &[u8]) -> u32 {
    u32::from(data.iter().fold(CHECKSUM_SEED, |acc, byte| acc ^ byte))
}

struct Response {
    op: u8,
    value: u32,
    data: Vec<u8>,
}

pub struct RomFlasher {
    port: Box<dyn SerialPort>,
    paced: bool,
}

impl RomFlasher {
    pub fn open(port_name: &str, paced: bool) -> Result<Self, String> {
        let port = serialport::new(port_name, BAUD)
            .timeout(Duration::from_millis(100))
            .open()
            .map_err(|error| format!("打开串口失败：{error}"))?;
        Ok(Self { port, paced })
    }

    /// 经典的 DTR/RTS 序列把芯片拉进下载模式，然后同步。
    pub fn connect(&mut self) -> Result<(), String> {
        let mut last_error = String::new();
        for _ in 0..5 {
            self.enter_bootloader()?;
            for _ in 0..7 {
                match self.sync() {
                    Ok(()) => {
                        // 同步后 ROM 会连回好几个 SYNC 应答，清干净。
                        std::thread::sleep(Duration::from_millis(50));
                        self.drain();
                        let magic = self.read_reg(CHIP_MAGIC_REG)?;
                        if magic != ESP32S3_MAGIC {
                            return Err(format!("不是 ESP32-S3（magic {magic:#x}）"));
                        }
                        return Ok(());
                    }
                    Err(error) => last_error = error,
                }
            }
        }
        Err(format!("无法与 ROM 下载程序同步：{last_error}"))
    }

    fn enter_bootloader(&mut self) -> Result<(), String> {
        // esptool 的 default_reset：EN 拉低，IO0 拉低的同时放开 EN。
        self.set_lines(false, true)?;
        std::thread::sleep(Duration::from_millis(100));
        self.set_lines(true, false)?;
        std::thread::sleep(Duration::from_millis(50));
        self.set_lines(false, false)?;
        std::thread::sleep(Duration::from_millis(50));
        self.drain();
        Ok(())
    }

    /// 烧完硬复位：EN 拉低再放开，IO0 保持高。
    pub fn hard_reset(&mut self) -> Result<(), String> {
        self.set_lines(false, true)?;
        std::thread::sleep(Duration::from_millis(100));
        self.set_lines(false, false)?;
        Ok(())
    }

    fn set_lines(&mut self, dtr: bool, rts: bool) -> Result<(), String> {
        self.port
            .write_data_terminal_ready(dtr)
            .and_then(|()| self.port.write_request_to_send(rts))
            .map_err(|error| format!("设置 DTR/RTS 失败：{error}"))
    }

    fn drain(&mut self) {
        let mut sink = [0_u8; 256];
        let _ = self.port.set_timeout(Duration::from_millis(20));
        while let Ok(count) = self.port.read(&mut sink) {
            if count == 0 {
                break;
            }
        }
        let _ = self.port.clear(serialport::ClearBuffer::All);
    }

    fn write_paced(&mut self, bytes: &[u8]) -> Result<(), String> {
        if !self.paced {
            self.port.write_all(bytes).map_err(|error| error.to_string())?;
            return self.port.flush().map_err(|error| error.to_string());
        }
        for piece in bytes.chunks(crate::serial_transport::PACE_PIECE_BYTES) {
            self.port.write_all(piece).map_err(|error| error.to_string())?;
            self.port.flush().map_err(|error| error.to_string())?;
            std::thread::sleep(crate::serial_transport::piece_delay(piece.len()));
        }
        Ok(())
    }

    /// 读一个 SLIP 帧，直到超时。
    fn read_frame(&mut self, deadline: Instant) -> Result<Vec<u8>, String> {
        let mut frame = Vec::new();
        let mut in_frame = false;
        let mut escaped = false;
        let mut byte = [0_u8; 1];
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| "等待应答超时".to_owned())?;
            let _ = self.port.set_timeout(remaining.min(Duration::from_millis(200)));
            match self.port.read(&mut byte) {
                Ok(1) => {}
                Ok(_) => continue,
                Err(error) if error.kind() == std::io::ErrorKind::TimedOut => continue,
                Err(error) => return Err(format!("串口读取失败：{error}")),
            }
            let value = byte[0];
            if !in_frame {
                if value == SLIP_END {
                    in_frame = true;
                }
                continue;
            }
            if escaped {
                frame.push(match value {
                    SLIP_ESC_END => SLIP_END,
                    SLIP_ESC_ESC => SLIP_ESC,
                    other => other,
                });
                escaped = false;
            } else if value == SLIP_ESC {
                escaped = true;
            } else if value == SLIP_END {
                if frame.is_empty() {
                    continue;
                }
                return Ok(frame);
            } else {
                frame.push(value);
            }
        }
    }

    fn read_response(&mut self, op: u8, timeout: Duration) -> Result<Response, String> {
        let deadline = Instant::now() + timeout;
        loop {
            let frame = self.read_frame(deadline)?;
            if frame.len() < 8 || frame[0] != 0x01 {
                continue;
            }
            let response = Response {
                op: frame[1],
                value: u32::from_le_bytes([frame[4], frame[5], frame[6], frame[7]]),
                data: frame[8..].to_vec(),
            };
            if response.op != op {
                continue;
            }
            if response.data.len() >= STATUS_BYTES {
                let status = &response.data[response.data.len() - STATUS_BYTES..];
                if status[0] != 0 {
                    return Err(format!("ROM 返回错误 {:#04x}（命令 {op:#04x}）", status[1]));
                }
            }
            return Ok(response);
        }
    }

    fn call(&mut self, op: u8, data: &[u8], checksum: u32, timeout: Duration) -> Result<Response, String> {
        let started = Instant::now();
        self.write_paced(&slip_encode(&command(op, data, checksum)))?;
        let written = started.elapsed();
        let response = self.read_response(op, timeout);
        tracing::debug!(
            op,
            write_ms = written.as_millis() as u64,
            wait_ms = (started.elapsed() - written).as_millis() as u64,
            "ROM 命令"
        );
        response
    }

    fn sync(&mut self) -> Result<(), String> {
        let mut data = vec![0x07, 0x07, 0x12, 0x20];
        data.extend(std::iter::repeat_n(0x55, 32));
        self.call(OP_SYNC, &data, 0, Duration::from_millis(100)).map(|_| ())
    }

    fn read_reg(&mut self, address: u32) -> Result<u32, String> {
        self.call(OP_READ_REG, &address.to_le_bytes(), 0, Duration::from_secs(3))
            .map(|response| response.value)
    }

    /// 让 ROM 挂上 SPI flash 并告诉它 16 MB 的参数。
    pub fn prepare_flash(&mut self) -> Result<(), String> {
        let mut attach = Vec::new();
        attach.extend_from_slice(&0_u32.to_le_bytes());
        attach.extend_from_slice(&0_u32.to_le_bytes());
        self.call(OP_SPI_ATTACH, &attach, 0, Duration::from_secs(3))?;
        let mut params = Vec::new();
        for value in [0_u32, 16 * 1024 * 1024, 64 * 1024, 4 * 1024, 256, 0xFFFF] {
            params.extend_from_slice(&value.to_le_bytes());
        }
        self.call(OP_SPI_SET_PARAMS, &params, 0, Duration::from_secs(3)).map(|_| ())
    }

    pub fn write_segment(&mut self, segment: &Segment, progress: &mut Progress<'_>) -> Result<(), String> {
        let size = segment.data.len();
        let blocks = size.div_ceil(BLOCK_BYTES);
        let started = Instant::now();
        let mut begin = Vec::new();
        for value in [size as u32, blocks as u32, BLOCK_BYTES as u32, segment.address, 0] {
            begin.extend_from_slice(&value.to_le_bytes());
        }
        // 擦除按每 MB 约 40 秒放宽超时，esptool 也是这个数。
        let erase_timeout = Duration::from_secs(10 + 40 * (size as u64 / (1024 * 1024) + 1));
        self.call(OP_FLASH_BEGIN, &begin, 0, erase_timeout)?;
        for (index, block) in segment.data.chunks(BLOCK_BYTES).enumerate() {
            let mut padded = block.to_vec();
            padded.resize(BLOCK_BYTES, 0xFF);
            let mut data = Vec::with_capacity(16 + BLOCK_BYTES);
            for value in [BLOCK_BYTES as u32, index as u32, 0, 0] {
                data.extend_from_slice(&value.to_le_bytes());
            }
            data.extend_from_slice(&padded);
            self.call(OP_FLASH_DATA, &data, checksum(&padded), Duration::from_secs(5))?;
            if (index + 1) % 256 == 0 {
                tracing::info!(
                    address = format_args!("{:#x}", segment.address),
                    blocks = index + 1,
                    ms_per_block = started.elapsed().as_millis() as u64 / (index as u64 + 1),
                    "烧录进度"
                );
            }
            (progress.on_progress)(
                (index + 1) as f32 / blocks as f32,
                &format!(
                    "{:#x} 已写 {}/{} 块，{:.1} s",
                    segment.address,
                    index + 1,
                    blocks,
                    started.elapsed().as_secs_f32()
                ),
            );
        }
        // 不发 FLASH_END：esptool 对 ROM 下载程序也不发，那会让它退出去跑
        // 用户代码；写完直接校验，最后统一硬复位。
        self.verify(segment)
    }

    fn verify(&mut self, segment: &Segment) -> Result<(), String> {
        let mut data = Vec::new();
        for value in [segment.address, segment.data.len() as u32, 0, 0] {
            data.extend_from_slice(&value.to_le_bytes());
        }
        let timeout = Duration::from_secs(8 * (segment.data.len() as u64 / (1024 * 1024) + 1) + 5);
        let response = self.call(OP_SPI_FLASH_MD5, &data, 0, timeout)?;
        // ROM 回 32 个十六进制字符，后面跟状态字节。
        let digest = response.data.get(..32).ok_or("MD5 应答过短")?;
        let actual = String::from_utf8_lossy(digest).to_ascii_lowercase();
        let expected = format!("{:x}", <md5::Md5 as md5::Digest>::digest(&segment.data));
        if actual != expected {
            return Err(format!("{:#x} 处校验不符：flash {actual}，文件 {expected}", segment.address));
        }
        Ok(())
    }
}

/// 整个流程：进下载模式、认芯片、挂 flash、逐段写并校验、硬复位。
pub fn flash(port_name: &str, paced: bool, segments: &[Segment], on_progress: &mut dyn FnMut(f32, &str)) -> Result<(), String> {
    let mut flasher = RomFlasher::open(port_name, paced)?;
    on_progress(0.0, "进入下载模式");
    flasher.connect()?;
    flasher.prepare_flash()?;
    let total: usize = segments.iter().map(|segment| segment.data.len()).sum();
    let mut done = 0_usize;
    for segment in segments {
        let base = done;
        let mut progress = Progress {
            on_progress: &mut |fraction, message| {
                let written = base + (fraction * segment.data.len() as f32) as usize;
                on_progress(written as f32 / total as f32, message);
            },
        };
        flasher.write_segment(segment, &mut progress)?;
        done += segment.data.len();
    }
    on_progress(1.0, "校验通过，重启设备");
    flasher.hard_reset()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slip_escapes_end_and_escape_bytes() {
        assert_eq!(slip_encode(&[0x01, 0xC0, 0xDB, 0x02]), vec![0xC0, 0x01, 0xDB, 0xDC, 0xDB, 0xDD, 0x02, 0xC0]);
    }

    #[test]
    fn a_command_packet_has_the_esptool_header() {
        let packet = command(OP_READ_REG, &0x4000_1000_u32.to_le_bytes(), 0);
        assert_eq!(&packet[..8], &[0x00, 0x0A, 0x04, 0x00, 0, 0, 0, 0]);
        assert_eq!(&packet[8..], &[0x00, 0x10, 0x00, 0x40]);
    }

    #[test]
    fn the_flash_data_checksum_xors_with_the_seed() {
        assert_eq!(checksum(&[]), 0xEF);
        assert_eq!(checksum(&[0xEF]), 0);
        assert_eq!(checksum(&[0x01, 0x02]), 0xEF ^ 0x03);
    }
}
