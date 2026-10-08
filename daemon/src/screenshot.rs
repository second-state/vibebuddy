//! 设备截图：发 `device.screenshot`，收行程编码的帧缓冲（`SHOT BEGIN` /
//! 若干 `SHOT` 行 / `SHOT END`），转成 PNG。格式与 tools/device-screenshot.py 一致。

use std::sync::Arc;
use std::time::Duration;

use vibebuddy_protocol::Event;
use tokio::sync::broadcast;

use crate::serial_transport::{DeviceMessage, Transport};

pub const WIDTH: usize = 320;
pub const HEIGHT: usize = 240;
/// 桥接下一帧要走十几秒；再留些余量给设备当时正在忙的事。
const TIMEOUT: Duration = Duration::from_secs(60);

pub struct Frame {
    pub width: usize,
    pub height: usize,
    /// RGB888，按行。
    pub pixels: Vec<u8>,
    pub backlight_on: bool,
}

fn rgb565_to_rgb(color: u16) -> [u8; 3] {
    let red = ((color >> 11) & 0x1F) as u32;
    let green = ((color >> 5) & 0x3F) as u32;
    let blue = (color & 0x1F) as u32;
    [
        (red * 255 / 31) as u8,
        (green * 255 / 63) as u8,
        (blue * 255 / 31) as u8,
    ]
}

/// 把 `SHOT` 行里的 `rgb565:长度` 段展开进像素缓冲。
pub fn decode_runs(line: &str, pixels: &mut Vec<u8>) -> Result<(), String> {
    for run in line.split_whitespace() {
        let (color, count) = run
            .split_once(':')
            .ok_or_else(|| format!("坏的行程段：{run}"))?;
        let color = u16::from_str_radix(color, 16).map_err(|_| format!("坏的颜色：{color}"))?;
        let count: usize = count.parse().map_err(|_| format!("坏的长度：{count}"))?;
        let rgb = rgb565_to_rgb(color);
        for _ in 0..count {
            pixels.extend_from_slice(&rgb);
        }
        if pixels.len() > WIDTH * HEIGHT * 3 {
            return Err("像素多于一屏".to_owned());
        }
    }
    Ok(())
}

pub fn encode_png(frame: &Frame) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut output, frame.width as u32, frame.height as u32);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
        let pixels = if frame.backlight_on {
            frame.pixels.clone()
        } else {
            // 背光关着时画面本来就看不见；压暗成一张"熄灯"的图，而不是假装亮着。
            frame.pixels.iter().map(|value| value / 4).collect()
        };
        writer.write_image_data(&pixels).map_err(|error| error.to_string())?;
    }
    Ok(output)
}

/// 抓一帧。要求调用方已经订阅了设备消息，免得错过第一行。
pub async fn capture(
    transport: Arc<dyn Transport>,
    mut bus: broadcast::Receiver<DeviceMessage>,
) -> Result<Frame, String> {
    let frame = Event::named("device.screenshot")
        .to_ndjson()
        .map_err(|error| error.to_string())?;
    transport.send(frame).map_err(|error| format!("{error:?}"))?;

    let deadline = tokio::time::Instant::now() + TIMEOUT;
    let mut pixels = Vec::with_capacity(WIDTH * HEIGHT * 3);
    let mut started = false;
    let mut backlight_on = true;
    let (mut width, mut height) = (WIDTH, HEIGHT);
    loop {
        let message = match tokio::time::timeout_at(deadline, bus.recv()).await {
            Ok(Ok(message)) => message,
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => return Err("设备消息积压，截图行丢失".to_owned()),
            Ok(Err(broadcast::error::RecvError::Closed)) => return Err("设备消息通道已关闭".to_owned()),
            Err(_) => return Err("等待截图超时".to_owned()),
        };
        let line = match message {
            DeviceMessage::Line(line) => line,
            DeviceMessage::Disconnected => return Err("链路断开".to_owned()),
            _ => continue,
        };
        if let Some(header) = line.strip_prefix("SHOT BEGIN") {
            (width, height) = match header.split_whitespace().next() {
                Some("320x240") => (320, 240),
                Some("240x240") => (240, 240),
                _ => return Err("未知屏幕尺寸".to_owned()),
            };
            started = true;
            backlight_on = !header.contains("BACKLIGHT OFF");
            pixels.clear();
            continue;
        }
        if line == "SHOT END" {
            if !started {
                continue;
            }
            if pixels.len() != width * height * 3 {
                return Err(format!("帧不完整：{} 像素", pixels.len() / 3));
            }
            return Ok(Frame { pixels, backlight_on, width, height });
        }
        if started && let Some(runs) = line.strip_prefix("SHOT ") {
            decode_runs(runs, &mut pixels)?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_expand_to_rgb_pixels() {
        let mut pixels = Vec::new();
        decode_runs("f800:2 07e0:1", &mut pixels).expect("合法的行程");
        assert_eq!(pixels, vec![255, 0, 0, 255, 0, 0, 0, 255, 0]);
        assert!(decode_runs("zz:1", &mut pixels).is_err());
        assert!(decode_runs("ffff", &mut pixels).is_err());
    }

    #[test]
    fn a_full_frame_encodes_as_png() {
        for width in [320, 240] {
            let frame = Frame { pixels: vec![0x80; width * HEIGHT * 3], backlight_on: true, width, height: HEIGHT };
            let png = encode_png(&frame).expect("编码 PNG");
            assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
            assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), width as u32);
            assert_eq!(u32::from_be_bytes(png[20..24].try_into().unwrap()), HEIGHT as u32);
        }
    }
}
