//! Flashes firmware over the ESP32-S3 ROM serial download protocol, without loading a stub.
//!
//! Not espflash: its ROM write block is fixed at 1 KB and its serial object is a concrete type we can't wrap,
//! while the BOX's CH343 bridge only swallows about two hundred bytes at a time (see the chunking note in
//! serial_transport; the earlier flashing script only worked after cutting esptool's block to 0x100). Blocks here are 256 bytes,
//! written in 128-byte chunks paced at line rate over the bridge. The protocol follows esptool: SLIP framing, SYNC, READ_REG
//! to identify the chip, SPI_ATTACH, FLASH_BEGIN/DATA/END, SPI_FLASH_MD5 verification, RTS hard reset.

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
/// ROM loader replies end with 4 status bytes (the stub uses 2).
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

/// SLIP framing.
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

/// Command packet: direction 0, opcode, data length, checksum (only FLASH_DATA uses it), data.
pub fn command(op: u8, data: &[u8], checksum: u32) -> Vec<u8> {
    let mut packet = Vec::with_capacity(8 + data.len());
    packet.push(0x00);
    packet.push(op);
    packet.extend_from_slice(&(data.len() as u16).to_le_bytes());
    packet.extend_from_slice(&checksum.to_le_bytes());
    packet.extend_from_slice(data);
    packet
}

/// FLASH_DATA checksum: XOR of all data bytes, seeded with 0xEF.
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
    usb_jtag: bool,
}

impl RomFlasher {
    pub fn open(port_name: &str, paced: bool) -> Result<Self, String> {
        let port = serialport::new(port_name, BAUD)
            .timeout(Duration::from_millis(100))
            .open()
            .map_err(|error| format!("failed to open serial port: {error}"))?;
        let usb_jtag = serialport::available_ports()
            .map_err(|error| format!("failed to inspect serial ports: {error}"))?
            .into_iter()
            .any(|candidate| {
                candidate.port_name == port_name
                    && matches!(candidate.port_type, serialport::SerialPortType::UsbPort(info) if (info.vid, info.pid) == (0x303A, 0x1001))
            });
        Ok(Self { port, paced, usb_jtag })
    }

    /// Selects the reset sequence for the port, enters download mode, then syncs.
    pub fn connect(&mut self) -> Result<(), String> {
        let mut last_error = String::new();
        for _ in 0..5 {
            self.enter_bootloader()?;
            for _ in 0..7 {
                match self.sync() {
                    Ok(()) => {
                        // After syncing, the ROM sends back several more SYNC replies; drain them.
                        std::thread::sleep(Duration::from_millis(50));
                        self.drain();
                        let magic = self.read_reg(CHIP_MAGIC_REG)?;
                        if magic != ESP32S3_MAGIC {
                            return Err(format!("not an ESP32-S3 (magic {magic:#x})"));
                        }
                        return Ok(());
                    }
                    Err(error) => last_error = error,
                }
            }
        }
        Err(format!("cannot sync with the ROM loader: {last_error}"))
    }

    fn enter_bootloader(&mut self) -> Result<(), String> {
        if self.usb_jtag {
            // Espressif USB Serial/JTAG must pass through DTR=RTS=true when resetting into download mode.
            // Match espflash's UsbJtagSerialReset; the UART bridge's classic sequence only restarts this port.
            self.set_rts(false)?;
            self.set_dtr(false)?;
            std::thread::sleep(Duration::from_millis(100));
            self.set_lines(true, false)?;
            std::thread::sleep(Duration::from_millis(100));
            self.set_rts(true)?;
            self.set_dtr(false)?;
            self.set_rts(true)?;
            std::thread::sleep(Duration::from_millis(100));
            self.set_lines(false, false)?;
        } else {
            // esptool's default_reset: pull EN low, then release EN while holding IO0 low.
            self.set_lines(false, true)?;
            std::thread::sleep(Duration::from_millis(100));
            self.set_lines(true, false)?;
            std::thread::sleep(Duration::from_millis(50));
            self.set_lines(false, false)?;
        }
        std::thread::sleep(Duration::from_millis(50));
        self.drain();
        Ok(())
    }

    /// Hard reset after flashing: pull EN low and release it, keeping IO0 high.
    pub fn hard_reset(&mut self) -> Result<(), String> {
        self.set_lines(false, true)?;
        std::thread::sleep(Duration::from_millis(100));
        self.set_lines(false, false)?;
        Ok(())
    }

    fn set_lines(&mut self, dtr: bool, rts: bool) -> Result<(), String> {
        self.set_dtr(dtr)?;
        self.set_rts(rts)
    }

    fn set_dtr(&mut self, dtr: bool) -> Result<(), String> {
        self.port.write_data_terminal_ready(dtr).map_err(|error| format!("failed to set DTR: {error}"))
    }

    fn set_rts(&mut self, rts: bool) -> Result<(), String> {
        self.port.write_request_to_send(rts).map_err(|error| format!("failed to set RTS: {error}"))
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

    /// Reads one SLIP frame, until the timeout.
    fn read_frame(&mut self, deadline: Instant) -> Result<Vec<u8>, String> {
        let mut frame = Vec::new();
        let mut in_frame = false;
        let mut escaped = false;
        let mut byte = [0_u8; 1];
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| "timed out waiting for a reply".to_owned())?;
            let _ = self.port.set_timeout(remaining.min(Duration::from_millis(200)));
            match self.port.read(&mut byte) {
                Ok(1) => {}
                Ok(_) => continue,
                Err(error) if error.kind() == std::io::ErrorKind::TimedOut => continue,
                Err(error) => return Err(format!("serial read failed: {error}")),
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
                    return Err(format!("ROM returned error {:#04x} (command {op:#04x})", status[1]));
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
            "ROM command"
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

    /// Has the ROM attach the SPI flash and tells it the 16 MB parameters.
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
        // Erasing gets about 40 seconds of extra timeout per MB, the same figure esptool uses.
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
                    "flash progress"
                );
            }
            (progress.on_progress)(
                (index + 1) as f32 / blocks as f32,
                &format!(
                    "{:#x}: wrote {}/{} blocks, {:.1} s",
                    segment.address,
                    index + 1,
                    blocks,
                    started.elapsed().as_secs_f32()
                ),
            );
        }
        // No FLASH_END: esptool doesn't send it to the ROM loader either, since it would exit to run
        // user code; verify right after writing and hard-reset once at the end.
        self.verify(segment)
    }

    fn verify(&mut self, segment: &Segment) -> Result<(), String> {
        let mut data = Vec::new();
        for value in [segment.address, segment.data.len() as u32, 0, 0] {
            data.extend_from_slice(&value.to_le_bytes());
        }
        let timeout = Duration::from_secs(8 * (segment.data.len() as u64 / (1024 * 1024) + 1) + 5);
        let response = self.call(OP_SPI_FLASH_MD5, &data, 0, timeout)?;
        // The ROM replies with 32 hex characters followed by the status bytes.
        let digest = response.data.get(..32).ok_or("MD5 reply too short")?;
        let actual = String::from_utf8_lossy(digest).to_ascii_lowercase();
        let expected = format!("{:x}", <md5::Md5 as md5::Digest>::digest(&segment.data));
        if actual != expected {
            return Err(format!("verify mismatch at {:#x}: flash {actual}, file {expected}", segment.address));
        }
        Ok(())
    }
}

/// The otadata partition in a partition table image, as a blank segment: writing it makes the bootloader boot
/// ota_0, where a USB flash puts the app, rather than an older image in ota_1 (ADR-0013). None for a table
/// without one, such as the single-slot layout before it.
pub fn blank_otadata(table: &[u8]) -> Option<Segment> {
    const ENTRY: usize = 32;
    const MAGIC: [u8; 2] = [0xAA, 0x50];
    const DATA: u8 = 0x01;
    const OTA: u8 = 0x00;
    table.as_chunks::<ENTRY>().0.iter().take_while(|entry| entry[..2] == MAGIC).find_map(|entry| {
        (entry[2] == DATA && entry[3] == OTA).then(|| {
            let address = u32::from_le_bytes(entry[4..8].try_into().unwrap());
            let size = u32::from_le_bytes(entry[8..12].try_into().unwrap());
            Segment { address, data: vec![0xFF; size as usize] }
        })
    })
}

/// The whole flow: enter download mode, identify the chip, attach flash, write and verify each segment, hard reset.
pub fn flash(port_name: &str, paced: bool, segments: &[Segment], on_progress: &mut dyn FnMut(f32, &str)) -> Result<(), String> {
    let mut flasher = RomFlasher::open(port_name, paced)?;
    on_progress(0.0, "entering download mode");
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
    on_progress(1.0, "verified, restarting device");
    flasher.hard_reset()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires exclusive access to an ESP32-S3 native USB device"]
    fn native_usb_enters_rom_download_mode() {
        let port = std::env::var("VIBEBUDDY_TEST_PORT").expect("set VIBEBUDDY_TEST_PORT to the native USB port");
        let mut flasher = RomFlasher::open(&port, false).expect("open the device");
        flasher.connect().expect("enter ROM download mode and read the ESP32-S3 chip ID");
        flasher.hard_reset().expect("restart the device after the check");
    }

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

    /// One partition table entry the way gen_esp32part.py lays it out.
    fn entry(kind: u8, subtype: u8, offset: u32, size: u32) -> Vec<u8> {
        let mut entry = vec![0xAA, 0x50, kind, subtype];
        entry.extend_from_slice(&offset.to_le_bytes());
        entry.extend_from_slice(&size.to_le_bytes());
        entry.resize(32, 0);
        entry
    }

    #[test]
    fn blank_otadata_finds_the_ota_data_partition() {
        let mut table = [entry(0x01, 0x02, 0x9000, 0x6000), entry(0x00, 0x10, 0x10000, 0x40_0000), entry(0x01, 0x00, 0xA1_0000, 0x2000)].concat();
        table.extend_from_slice(&[0xEB, 0xEB]);
        table.resize(0xC00, 0xFF);
        let segment = blank_otadata(&table).expect("an otadata segment");
        assert_eq!(segment.address, 0xA1_0000);
        assert_eq!(segment.data, vec![0xFF; 0x2000]);
    }

    #[test]
    fn a_single_slot_table_has_no_otadata() {
        let mut table = [entry(0x01, 0x02, 0x9000, 0x6000), entry(0x00, 0x00, 0x10000, 0x40_0000), entry(0x01, 0x40, 0x41_0000, 0x20_0000)].concat();
        table.resize(0xC00, 0xFF);
        assert!(blank_otadata(&table).is_none());
    }

    #[test]
    fn blank_otadata_reads_the_real_table() {
        let csv = concat!(env!("CARGO_MANIFEST_DIR"), "/../firmware/partitions.csv");
        let out = std::env::temp_dir().join(format!("vibebuddy-pt-{}.bin", std::process::id()));
        let status = std::process::Command::new("python3")
            .args([concat!(env!("CARGO_MANIFEST_DIR"), "/../tools/make-partition-table.py"), csv, out.to_str().unwrap()])
            .output()
            .expect("run make-partition-table.py");
        assert!(status.status.success());
        let table = std::fs::read(&out).unwrap();
        std::fs::remove_file(&out).ok();
        assert_eq!(blank_otadata(&table).map(|segment| (segment.address, segment.data.len())), Some((0xA1_0000, 0x2000)));
    }
}
