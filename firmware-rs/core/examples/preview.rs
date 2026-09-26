//! 在 Mac 上把固件画面渲染成 PPM，和 C 固件的 display_preview.c 一模一样的
//! 场景与文件名：`tools/compare-display.sh` 拿两边的输出逐字节比对。
//!
//! 用法：cargo run -p vibebuddy-firmware-core --example preview -- <目录> [leisure]

use std::fs;
use std::path::{Path, PathBuf};

use vibebuddy_firmware_core::canvas::{FRAME_BYTES, HEIGHT, WIDTH};
use vibebuddy_firmware_core::display::{Display, Mode, Scene, Screen, State, TaskInput};
use vibebuddy_firmware_core::leisure::{self, Leisure, Skit};
use vibebuddy_firmware_core::pomodoro::{Pomodoro, Tally};

struct Frame {
    bytes: Vec<u8>,
}

impl Screen for Frame {
    fn frame(&mut self) -> &mut [u8] {
        &mut self.bytes
    }
    fn present(&mut self) -> Result<(), ()> {
        Ok(())
    }
    fn set_backlight(&mut self, _on: bool) -> Result<(), ()> {
        Ok(())
    }
}

struct Preview {
    directory: PathBuf,
    screen: Frame,
    display: Display,
    pomodoro: Pomodoro,
    leisure: Leisure,
    /// C 预览里时钟按 FreeRTOS 的 10 ms 节拍走，毫秒数要先截到 10 的倍数。
    now: u32,
}

macro_rules! scene {
    ($preview:ident) => {
        Scene { now_ms: $preview.now, pomodoro: &$preview.pomodoro, leisure: &$preview.leisure }
    };
}

impl Preview {
    fn set_ms(&mut self, ms: u32) {
        self.now = ms / 10 * 10;
    }

    fn snapshot(&mut self, name: &str) {
        self.display.refresh(&mut self.screen, &scene!(self));
        write_ppm(&self.directory.join(format!("{name}.ppm")), &self.screen.bytes);
    }

    fn set_mode(&mut self, mode: Mode) {
        self.display.set_mode(&mut self.screen, &scene!(self), mode);
    }

    fn show(&mut self, state: State, title: Option<&[u8]>, tasks: &[TaskInput]) {
        let _ = self.display.show_tasks(&mut self.screen, &scene!(self), state, title, tasks);
    }
}

fn write_ppm(path: &Path, bytes: &[u8]) {
    let mut out = format!("P6\n{WIDTH} {HEIGHT}\n255\n").into_bytes();
    for pair in bytes[..FRAME_BYTES].as_chunks::<2>().0 {
        let pixel = u16::from_be_bytes([pair[0], pair[1]]) as u32;
        out.push((((pixel >> 11) & 0x1f) * 255 / 31) as u8);
        out.push((((pixel >> 5) & 0x3f) * 255 / 63) as u8);
        out.push(((pixel & 0x1f) * 255 / 31) as u8);
    }
    fs::write(path, out).expect("写 PPM");
}

/// 把每个剧目逐帧渲染出来。
fn render_skits(preview: &mut Preview) {
    let skits = [
        (Skit::Patrol, "patrol", 96),
        (Skit::Ball, "ball", 96),
        (Skit::Read, "read", 120),
        (Skit::Stars, "stars", 120),
        (Skit::Hide, "hide", 80),
        (Skit::Startle, "startle", 80),
        (Skit::Dream, "dream", 96),
        (Skit::Sleep, "sleep", 48),
        (Skit::None, "rest", 48),
    ];
    for (skit, name, frames) in skits {
        let start = 100_000;
        preview.leisure = Leisure::new(1, start);
        preview.leisure.start_skit(skit, start);
        preview.set_mode(Mode::Duty);
        preview.set_mode(Mode::Leisure);
        for frame in 0..frames {
            preview.set_ms(start + frame * leisure::FRAME_MS);
            preview.snapshot(&format!("skit_{name}_{frame:03}"));
        }
    }
}

fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    let directory = PathBuf::from(arguments.get(1).expect("用法: preview <输出目录> [leisure]"));
    fs::create_dir_all(&directory).expect("建目录");
    let mut preview = Preview {
        directory,
        screen: Frame { bytes: vec![0; FRAME_BYTES] },
        display: Display::new(),
        pomodoro: Pomodoro::new(),
        leisure: Leisure::new(1, 0),
        now: 0,
    };
    preview.display.start(&mut preview.screen, &scene!(preview)).unwrap();
    preview.display.set_firmware_build(&mut preview.screen, &scene!(preview), b"9b642af 2026-09-14 17:41");
    preview.display.set_daemon_build(&mut preview.screen, &scene!(preview), b"9b642af 2026-09-14 17:43");
    preview.display.set_stats(&[b"7 DONE", b"4 ASKS", b"1H23 BUSY"]);
    preview.pomodoro.restore_tally(Tally { day: 20260915, completed: 3, focus_s: 4500 });
    if arguments.get(2).map(String::as_str) == Some("leisure") {
        render_skits(&mut preview);
        return;
    }

    preview.set_ms(1000);
    preview.set_mode(Mode::Pomodoro);
    preview.snapshot("pomodoro_idle");

    let tasks = [
        TaskInput { title: Some(b"CC:POMODORO TIMER FEATURE"), state: State::Working, elapsed_s: 75, project: Some(b"AGENT-BEACON") },
        TaskInput {
            title: Some(b"CX:EROS-TRAINING-INFRA"),
            state: State::InputRequired,
            elapsed_s: 900,
            project: Some(b"EROS-TRAINING-INFRA"),
        },
    ];
    preview.show(State::InputRequired, Some(b"CC:AGENT-BEACON"), &tasks);
    preview.pomodoro.toggle(1000);
    preview.set_ms(1000 + 6 * 60 * 1000 + 39 * 1000);
    preview.snapshot("pomodoro_focus");

    preview.pomodoro.toggle(1000 + 6 * 60 * 1000 + 39 * 1000);
    preview.display.preview_pose(Some(1), None, None);
    preview.snapshot("pomodoro_paused_blink");
    preview.display.preview_pose(Some(0), None, None);
    preview.snapshot("pomodoro_paused");
    preview.pomodoro.toggle(1000 + 6 * 60 * 1000 + 39 * 1000);

    preview.set_mode(Mode::Duty);
    preview.snapshot("pet_with_badge");

    let focus_end = 1000 + 25 * 60 * 1000 + 100;
    preview.set_ms(focus_end);
    preview.pomodoro.tick(focus_end);
    preview.display.pomodoro_ended(&scene!(preview));
    preview.set_mode(Mode::Pomodoro);
    preview.show(State::Idle, None, &[]);

    // 闹铃头三秒逐帧：抖动 20 帧各 100 ms，之后脉动每拍 500 ms。
    for frame in 0..30u32 {
        if frame < 20 {
            preview.display.preview_pose(Some(frame), Some(20 - frame), None);
        } else {
            preview.display.preview_pose(Some((frame - 20) / 5), Some(0), None);
        }
        preview.snapshot(&format!("pomodoro_alarm_{frame:03}"));
    }
    preview.display.preview_pose(Some(1), Some(20), None);
    preview.snapshot("pomodoro_alarm_shake");
    preview.display.preview_pose(None, Some(0), None);
    preview.snapshot("pomodoro_alarm_pulse_dim");
    preview.display.preview_pose(Some(0), None, None);
    preview.snapshot("pomodoro_alarm_pulse");
    // 待开始的样子本身也要看，先把闹铃按掉。
    preview.display.preview_pose(None, None, Some(false));
    preview.snapshot("pomodoro_break_pending");

    preview.pomodoro.toggle(focus_end + 30000);
    preview.set_ms(focus_end + 30000 + 2 * 60 * 1000);
    preview.snapshot("pomodoro_break");

    preview.display.set_link_lost(&mut preview.screen, &scene!(preview), true);
    preview.snapshot("pomodoro_no_link");
    preview.display.set_link_lost(&mut preview.screen, &scene!(preview), false);
    preview.display.set_muted(&mut preview.screen, &scene!(preview), true);
    preview.set_mode(Mode::Duty);
    preview.snapshot("pet_muted");
}
