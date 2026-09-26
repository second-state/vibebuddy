//! 固件的主程序：串口行协议、按键、番茄钟、休闲、语音、屏幕之间的调度。
//! 对应 C 固件的 vibebuddy_fw.c。硬件都藏在 [`Board`] 后面，所以整条链在
//! Mac 上就能跑测试：喂一行 JSON，看它回了什么、画了什么、播了什么。
//!
//! 串口上的每一行输出都和 C 固件逐字一致，Mac 端的解析不用改。

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use serde_json::{Map, Value};
use vibebuddy_protocol::{Event, ProtocolError, VERSION};

use crate::audio::{Prompt, clamp_volume, VOLUME_DEFAULT};
use crate::buttons::{ButtonEvent, Buttons, Levels};
use crate::display::{Display, MAX_STATS, MAX_TASKS, Mode, Scene, Screen, State, TaskInput};
use crate::leisure::{Leisure, Tier};
use crate::pomodoro::{Phase, Pomodoro, Run, Transition};
use crate::storage::{Flash, Settings, SettingsStore, find_partition};
use crate::text;
use crate::voice_pack::crc32;
use crate::voices::{ClipTable, Voices, VoiceError};

pub const MAX_LINE_BYTES: usize = 1024;
const LINE_BUFFER_BYTES: usize = MAX_LINE_BYTES + 2;
/// 超过这个时间没有收到任何消息，就认为与 Mac 端失联。
const LINK_TIMEOUT_MS: u32 = 15000;
/// task.done 之后这么久自己回到空闲。
const DONE_TO_IDLE_MS: u32 = 5000;
/// voice.begin 最多等这么久让正在播的一句放完；最长的一句不到 7 秒。
const AUDIO_DRAIN_MS: u32 = 10000;

pub const FIRMWARE_NAME: &str = "vibebuddy-fw 0.1.0";

/// 音频初始化的结果：成功时是 codec 型号（ES8311 或 NS4168），失败时是
/// 卡在哪一步。两者都原样报给 Mac 端。
pub type AudioStatus = Result<&'static str, &'static str>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VolumeError {
    /// NS4168 版本没有 codec，音量不可调。
    NotSupported,
    Failed,
}

/// 截图时的回调：拿到只读的帧缓冲和一个往串口写的口子。
pub type FrameAction<'a> = dyn FnMut(&[u8], &mut dyn FnMut(&[u8])) + 'a;

/// 设备层提供的一切。
pub trait Board: Screen {
    /// 单调毫秒计数，允许回绕。
    fn now_ms(&self) -> u32;
    /// 同时写到 UART0 与 USB Serial/JTAG。只是诊断通道，谁都不许拖住主循环：
    /// 没有对端在读的那一路写不进去就丢。
    fn write(&mut self, bytes: &[u8]);
    fn flash(&mut self) -> &mut dyn Flash;
    /// 同时借出帧缓冲（只读）与串口输出：截图要一边读帧一边往外写。
    fn with_frame_and_output(&mut self, action: &mut FrameAction);

    /// 初始化屏幕硬件（背光保持关闭）。
    fn init_display(&mut self) -> bool;
    /// 初始化 I2S 与 codec，按给定音量开声。
    fn init_audio(&mut self, volume: u32) -> AudioStatus;
    /// 初始化按键，返回当时三个键的状态。
    fn init_buttons(&mut self) -> Option<Levels>;

    /// K0 的电平与扩展口上 K1、K2 的电平（true 为按下）；扩展口读失败给 None。
    fn read_buttons(&mut self) -> (bool, Option<(bool, bool)>);

    /// 排一句播报。`clips` 为 None 时播内置音色。队满返回 Err。
    fn play(&mut self, prompt: Prompt, clips: ClipTable) -> Result<(), ()>;
    /// 停掉正在播的并清空队列。
    fn stop_audio(&mut self);
    /// 还在出声：队列里有、正在播，或者 DMA 里还没冲成静音。
    fn audio_busy(&self) -> bool;
    fn set_volume(&mut self, level: u32) -> Result<(), VolumeError>;
}

pub struct Firmware {
    pomodoro: Pomodoro,
    leisure: Leisure,
    display: Display,
    voices: Voices,
    buttons: Option<Buttons>,
    settings: Option<SettingsStore>,
    volume: u32,
    audio_ready: bool,
    build: Vec<u8>,

    line: Vec<u8>,
    discarding: bool,
    last_message_ms: u32,
    link_lost: bool,
    ready_deadline: Option<u32>,
    /// 静音：长按 K2 翻转，不持久化。开会静了音忘记开回来，设备就哑好几天；
    /// 重启恢复有声比记住更安全，屏幕上的 MUTE 标记负责提醒。
    muted: bool,
    /// voice.begin 收到了，正在等播放停下来再擦分区：总字节数与开始等的时刻。
    pending_voice_begin: Option<(u32, u32)>,

    /// 上一次报给 Mac 端的档位、关灯状态与小时数，只在变化时各报一行。
    reported_tier: Tier,
    reported_lights_out: bool,
    reported_hour: i32,
}

/// 借用 Firmware 的番茄钟与休闲导演组一帧画面的上下文。
macro_rules! scene {
    ($self:ident, $now:expr) => {
        Scene { now_ms: $now, pomodoro: &$self.pomodoro, leisure: &$self.leisure }
    };
}

fn task_state(status: &str) -> State {
    match status {
        "input_required" => State::InputRequired,
        "done" => State::Done,
        "failed" => State::Failed,
        _ => State::Working,
    }
}

/// strtoul 的子集：跳过前导空白，读开头的十进制数字；没有数字就是 0。
fn leading_number(text: &str) -> u32 {
    text.trim_start().bytes().take_while(u8::is_ascii_digit).fold(0u32, |value, digit| {
        value.wrapping_mul(10).wrapping_add((digit - b'0') as u32)
    })
}

/// 信封的扩展字段（BTreeMap）与任务卡（JSON 对象）都能按键取值。
trait Fields {
    fn field(&self, key: &str) -> Option<&Value>;
}

impl Fields for Map<alloc::string::String, Value> {
    fn field(&self, key: &str) -> Option<&Value> {
        self.get(key)
    }
}

impl Fields for BTreeMap<alloc::string::String, Value> {
    fn field(&self, key: &str) -> Option<&Value> {
        self.get(key)
    }
}

fn number(fields: &impl Fields, key: &str) -> Option<f64> {
    fields.field(key).and_then(Value::as_f64)
}

fn string<'a>(fields: &'a impl Fields, key: &str) -> Option<&'a str> {
    fields.field(key).and_then(Value::as_str)
}

impl Firmware {
    pub fn new(seed: u32, now_ms: u32, build: &[u8]) -> Self {
        Self {
            pomodoro: Pomodoro::new(),
            leisure: Leisure::new(seed, now_ms),
            display: Display::new(),
            voices: Voices::new(),
            buttons: None,
            settings: None,
            volume: VOLUME_DEFAULT,
            audio_ready: false,
            build: build.to_vec(),
            line: Vec::with_capacity(LINE_BUFFER_BYTES),
            discarding: false,
            last_message_ms: now_ms,
            link_lost: false,
            ready_deadline: None,
            muted: false,
            pending_voice_begin: None,
            reported_tier: Tier::Alert,
            reported_lights_out: false,
            reported_hour: -1,
        }
    }

    fn write_literal<B: Board>(board: &mut B, text: &str) {
        board.write(text.as_bytes());
    }

    /// 一行 `label value`，value 里的回车换行换成空格。
    fn write_value_line<B: Board>(board: &mut B, label: &str, value: &[u8]) {
        board.write(label.as_bytes());
        let mut start = 0;
        for (index, &byte) in value.iter().enumerate() {
            if byte == b'\r' || byte == b'\n' {
                board.write(&value[start..index]);
                board.write(b" ");
                start = index + 1;
            }
        }
        board.write(&value[start..]);
        board.write(b"\n");
    }

    /// 开机：按 C 固件的顺序初始化各部分，并报同样的几行。
    pub fn boot<B: Board>(&mut self, board: &mut B) {
        let now = board.now_ms();
        // 昨天的记录也先恢复：换不换日要等心跳带来日期才知道。
        let settings_region = find_partition(board.flash(), "nvs");
        let loaded = settings_region.map(|region| SettingsStore::open(board.flash(), region));
        match loaded {
            Some(Ok((store, found))) => {
                let tally = found.map(|settings| settings.tally).unwrap_or_default();
                if let Some(settings) = found {
                    self.volume = clamp_volume(settings.volume);
                }
                self.pomodoro.restore_tally(tally);
                self.settings = Some(store);
                let line = text!(48, "TALLY LOADED {} {}S DAY {}\n", tally.completed % 10000, tally.focus_s % 1_000_000, tally.day % 100_000_000);
                board.write(line.as_bytes());
            }
            _ => Self::write_literal(board, "TALLY LOAD ERROR\n"),
        }

        let display_ok = board.init_display() && self.display.start(board, &scene!(self, now)).is_ok();
        if display_ok {
            let build = self.build.clone();
            self.display.set_firmware_build(board, &scene!(self, now), &build);
            Self::write_value_line(board, "DISPLAY READY BUILD ", &self.build);
        } else {
            Self::write_literal(board, "DISPLAY ERROR\n");
        }

        let voices_region = find_partition(board.flash(), "voices");
        match self.voices.init(board.flash(), voices_region) {
            Ok(()) => Self::write_value_line(board, "VOICES ", self.voices.current_id().as_bytes()),
            Err(_) => Self::write_literal(board, "VOICES NO PARTITION\n"),
        }

        match board.init_audio(self.volume) {
            Ok(codec) => {
                self.audio_ready = true;
                Self::write_literal(board, "AUDIO READY\n");
                Self::write_value_line(board, "AUDIO CODEC ", codec.as_bytes());
                self.announce_volume(board);
            }
            Err(step) => Self::write_value_line(board, "AUDIO ERROR ", step.as_bytes()),
        }

        match board.init_buttons() {
            Some(levels) => {
                self.buttons = Some(Buttons::new(levels, board.now_ms()));
                Self::write_literal(board, "BUTTONS READY\n");
            }
            None => Self::write_literal(board, "BUTTONS ERROR\n"),
        }

        self.last_message_ms = board.now_ms();
        Self::write_value_line(board, "READY ", FIRMWARE_NAME.as_bytes());
    }

    /// 串口收到的字节，UART 与 USB 两路都往这里送。
    pub fn receive<B: Board>(&mut self, board: &mut B, bytes: &[u8]) {
        for &byte in bytes {
            if byte == b'\n' {
                if self.discarding {
                    Self::write_literal(board, "ERROR input_too_large\n");
                } else {
                    if self.line.last() == Some(&b'\r') {
                        self.line.pop();
                    }
                    if self.line.len() > MAX_LINE_BYTES {
                        Self::write_literal(board, "ERROR input_too_large\n");
                    } else {
                        self.last_message_ms = board.now_ms();
                        self.set_link_lost(board, false);
                        let line = core::mem::take(&mut self.line);
                        self.handle_line(board, &line);
                        self.line = line;
                    }
                }
                self.line.clear();
                self.discarding = false;
                continue;
            }
            if !self.discarding {
                if self.line.len() == LINE_BUFFER_BYTES - 1 {
                    self.discarding = true;
                } else {
                    self.line.push(byte);
                }
            }
        }
    }

    /// 主循环每一圈调一次：失联检测、延时回到空闲、按键、番茄钟、休闲、动画。
    pub fn poll<B: Board>(&mut self, board: &mut B) {
        let now = board.now_ms();
        if now.wrapping_sub(self.last_message_ms) as i32 >= LINK_TIMEOUT_MS as i32 {
            self.set_link_lost(board, true);
        }
        if let Some(deadline) = self.ready_deadline
            && now.wrapping_sub(deadline) as i32 >= 0
        {
            self.ready_deadline = None;
            if self.display.show_tasks(board, &scene!(self, now), State::Idle, None, &[]).is_err() {
                Self::write_literal(board, "DISPLAY ERROR\n");
            }
        }
        self.continue_voice_begin(board);
        self.tick_buttons(board);
        let transition = self.pomodoro.tick(board.now_ms());
        self.handle_pomodoro_transition(board, transition);
        self.tend_leisure(board);
        let now = board.now_ms();
        self.display.tick(board, &scene!(self, now));
    }

    fn set_link_lost<B: Board>(&mut self, board: &mut B, lost: bool) {
        self.link_lost = lost;
        let now = board.now_ms();
        self.display.set_link_lost(board, &scene!(self, now), lost);
    }

    fn set_mode<B: Board>(&mut self, board: &mut B, mode: Mode) {
        if self.display.mode() == mode {
            return;
        }
        let now = board.now_ms();
        self.display.set_mode(board, &scene!(self, now), mode);
        Self::write_value_line(board, "MODE ", mode.name().as_bytes());
    }

    fn refresh<B: Board>(&mut self, board: &mut B) {
        let now = board.now_ms();
        self.display.refresh(board, &scene!(self, now));
    }

    fn report_pomodoro<B: Board>(board: &mut B, what: &str) {
        Self::write_value_line(board, "POMODORO ", what.as_bytes());
    }

    fn clips(&self) -> ClipTable {
        if self.pending_voice_begin.is_some() { None } else { self.voices.clips() }
    }

    /// 所有语音都从这里出去，静音时只记一行日志。
    fn play_prompt<B: Board>(&mut self, board: &mut B, prompt: Prompt, label: &str) {
        if self.muted {
            Self::write_value_line(board, "AUDIO MUTED ", label.as_bytes());
            return;
        }
        if !self.audio_ready || board.play(prompt, self.clips()).is_err() {
            Self::write_literal(board, "AUDIO ERROR\n");
        } else {
            Self::write_value_line(board, "AUDIO QUEUED ", label.as_bytes());
        }
    }

    /// 音量是设备自己的事实，App 的滑块只是遥控：改完报一行，hello 也报。
    fn announce_volume<B: Board>(&self, board: &mut B) {
        let line = text!(24, "VOLUME {}\n", self.volume % 1000);
        board.write(line.as_bytes());
    }

    fn save_settings<B: Board>(&mut self, board: &mut B) -> bool {
        let settings = Settings { tally: self.pomodoro.tally(), volume: self.volume };
        match self.settings.as_mut() {
            Some(store) => store.save(board.flash(), &settings).is_ok(),
            None => false,
        }
    }

    /// 当日记录变了就存一次，并报一行给 Mac 端。一天只有几次。
    fn save_tally<B: Board>(&mut self, board: &mut B) {
        let tally = self.pomodoro.tally();
        let line = text!(48, "POMODORO TODAY {} {}S DAY {}\n", tally.completed % 10000, tally.focus_s % 1_000_000, tally.day % 100_000_000);
        board.write(line.as_bytes());
        if !self.save_settings(board) {
            Self::write_literal(board, "TALLY SAVE ERROR\n");
        }
    }

    /// 把番茄钟推到前面来；已经在前面就只重绘。
    fn show_pomodoro<B: Board>(&mut self, board: &mut B) {
        if self.display.mode() == Mode::Pomodoro {
            self.refresh(board);
        } else {
            self.set_mode(board, Mode::Pomodoro);
        }
    }

    fn tick_buttons<B: Board>(&mut self, board: &mut B) {
        let Some(buttons) = self.buttons.as_mut() else {
            return;
        };
        let (k0, expander) = board.read_buttons();
        let events = buttons.update(k0, expander, board.now_ms());
        for event in events.into_iter().flatten() {
            self.on_button(board, event);
        }
    }

    /// 三个键各管一件事，与模式无关：K0 是番茄钟的键，K1 在值班与番茄钟之间
    /// 切换（长按去休闲），K2 交给 Mac 端去打开来源。休闲模式里任何键先把
    /// 小灯灵叫回值班，再执行本职：休闲没有遮住任何需要先看一眼的东西。
    fn on_button<B: Board>(&mut self, board: &mut B, event: ButtonEvent) {
        let now = board.now_ms();
        let mode_before = self.display.mode();
        self.leisure.note_activity(now);
        if mode_before == Mode::Leisure {
            self.leisure.tick(now);
            self.set_mode(board, Mode::Duty);
        }

        match event {
            ButtonEvent::K2Short => {
                Self::write_literal(board, "{\"version\":1,\"event\":\"button\",\"button\":\"K2\",\"action\":\"press\"}\n");
                return;
            }
            ButtonEvent::K2Long => {
                self.muted = !self.muted;
                let muted = self.muted;
                self.display.set_muted(board, &scene!(self, now), muted);
                Self::write_literal(board, if muted { "MUTE ON\n" } else { "MUTE OFF\n" });
                return;
            }
            ButtonEvent::K1Short => {
                self.set_mode(board, if mode_before == Mode::Pomodoro { Mode::Duty } else { Mode::Pomodoro });
                return;
            }
            ButtonEvent::K1Long => {
                self.leisure.force_bored(now);
                self.leisure.tick(now);
                self.set_mode(board, Mode::Leisure);
                return;
            }
            ButtonEvent::K0Short | ButtonEvent::K0Long => {}
        }

        let before = self.pomodoro.view(now);
        let break_phase = before.phase == Phase::Break;
        if event == ButtonEvent::K0Long {
            if before.is_idle() {
                return;
            }
            self.pomodoro.stop();
            Self::report_pomodoro(board, if break_phase && before.run == Run::Pending { "BREAK SKIPPED" } else { "STOPPED" });
            self.refresh(board);
            return;
        }
        self.pomodoro.toggle(now);
        if before.run == Run::Pending {
            Self::report_pomodoro(board, if break_phase { "BREAK START" } else { "FOCUS START" });
            // 开始一个阶段时把番茄钟推到前面：圆环开始走就是反馈。
            self.show_pomodoro(board);
            return;
        }
        // 暂停与继续不换场景，小灯灵场景右上角的徽章会跟着闪。
        Self::report_pomodoro(board, if before.run == Run::Paused { "RESUMED" } else { "PAUSED" });
        self.refresh(board);
    }

    /// 阶段结束是只消费一次的边沿：播一次语音，并把番茄钟推到前面来——
    /// 这正是用户该看一眼的时刻。下一阶段停在待开始，等用户按 K0。
    fn handle_pomodoro_transition<B: Board>(&mut self, board: &mut B, transition: Transition) {
        let focus_ended = match transition {
            Transition::Nothing => return,
            Transition::FocusEnded => true,
            Transition::BreakEnded => false,
        };
        Self::report_pomodoro(board, if focus_ended { "FOCUS END" } else { "BREAK END" });
        if focus_ended {
            self.save_tally(board);
        }
        if focus_ended {
            self.play_prompt(board, Prompt::FocusDone, "FOCUS_DONE");
        } else {
            self.play_prompt(board, Prompt::BreakDone, "BREAK_DONE");
        }
        let now = board.now_ms();
        self.display.pomodoro_ended(&scene!(self, now));
        self.show_pomodoro(board);
    }

    /// 无聊度按状态累计，不按消息：Agent 有活、番茄钟在走、链路断了，都不算
    /// 空闲。值班空闲够久就去休闲；有事立刻回来。番茄钟模式里，待开始的一屏
    /// 静止的 25:00 和值班空闲一样无聊，五分钟就走；暂停的是用户有意停在那里
    /// 的，放了半小时才当人走了。都是先回值班，接着自然会去休闲。
    fn tend_leisure<B: Board>(&mut self, board: &mut B) {
        let now = board.now_ms();
        let pomodoro = self.pomodoro.view(now);
        if !self.display.agent_idle() || pomodoro.run == Run::Running || self.link_lost {
            self.leisure.note_activity(now);
        }

        let mode = self.display.mode();
        let changed = self.leisure.tick(now);
        let leisure = self.leisure.view(now);
        // 档位按差异汇报，不按“本次有没有变化”：唤醒路径会先在别处推进导演。
        if leisure.tier != self.reported_tier {
            self.reported_tier = leisure.tier;
            Self::write_value_line(board, "LEISURE ", leisure.tier.name().as_bytes());
        } else if changed && mode == Mode::Leisure {
            Self::write_value_line(board, "LEISURE SKIT ", leisure.skit.name().as_bytes());
        }
        if leisure.lights_out != self.reported_lights_out {
            self.reported_lights_out = leisure.lights_out;
            Self::write_literal(board, if leisure.lights_out { "LEISURE LIGHTS OUT\n" } else { "LEISURE LIGHTS ON\n" });
        }

        if mode == Mode::Duty && leisure.tier != Tier::Alert {
            self.set_mode(board, Mode::Leisure);
        } else if mode == Mode::Leisure && leisure.tier == Tier::Alert {
            self.set_mode(board, Mode::Duty);
        } else if mode == Mode::Pomodoro {
            let pending = pomodoro.run == Run::Pending;
            if leisure.tier == Tier::Sleepy || (pending && leisure.tier == Tier::Bored) {
                self.set_mode(board, Mode::Duty);
            }
        }
    }

    /// 当日战绩随每条状态事件下发，空闲屏用它轮播。
    ///
    /// 不能只在 `agent.idle` 上取：`task.done` 之后设备是自己回到空闲的，
    /// 那一刻正是用户会看的一眼，缓存的战绩必须已经包含刚完成的这一件。
    fn parse_stats(&mut self, fields: &BTreeMap<alloc::string::String, Value>) {
        let Some(Value::Array(items)) = fields.get("stats") else {
            return;
        };
        let lines: Vec<&str> = items.iter().filter_map(Value::as_str).take(MAX_STATS).collect();
        let bytes: Vec<&[u8]> = lines.iter().map(|line| line.as_bytes()).collect();
        self.display.set_stats(&bytes);
        // “7 DONE” 这一行的数字：休闲时它决定小灯灵是累了还是无聊。
        let done = lines.iter().find(|line| line.contains("DONE")).map(|line| leading_number(line)).unwrap_or(0);
        self.leisure.set_done_count(done);
    }

    fn show_event<B: Board>(&mut self, board: &mut B, fields: &BTreeMap<alloc::string::String, Value>, event: &str, title: Option<&str>) {
        let task_items: Vec<&Map<alloc::string::String, Value>> = match fields.get("tasks") {
            Some(Value::Array(items)) => items.iter().filter_map(Value::as_object).collect(),
            _ => Vec::new(),
        };
        let mut tasks: Vec<TaskInput> = Vec::new();
        for item in task_items {
            if tasks.len() == MAX_TASKS {
                break;
            }
            let (Some(task_title), Some(status)) = (string(item, "title"), string(item, "status")) else {
                continue;
            };
            tasks.push(TaskInput {
                title: Some(task_title.as_bytes()),
                state: task_state(status),
                elapsed_s: number(item, "elapsed_s").map(|value| value as i32).unwrap_or(0),
                project: string(item, "project").map(str::as_bytes),
            });
        }

        let now = board.now_ms();
        let (state, state_label, mut prompt) = match event {
            "task.start" => (State::Working, "WORKING", None),
            "agent.idle" => (State::Idle, "READY", None),
            "agent.input_required" => (State::InputRequired, "INPUT REQUIRED", Some((Prompt::InputRequired, "INPUT_REQUIRED"))),
            "task.done" => (State::Done, "DONE", Some((Prompt::Done, "DONE"))),
            "task.error" | "agent.blocked" => (State::Failed, "FAILED", Some((Prompt::Failed, "FAILED"))),
            _ => return,
        };
        self.ready_deadline = if event == "task.done" { Some(now.wrapping_add(DONE_TO_IDLE_MS)) } else { None };

        if fields.get("suppress_audio") == Some(&Value::Bool(true)) {
            prompt = None;
        }
        match string(fields, "announcement") {
            Some("done") => prompt = Some((Prompt::Done, "DONE")),
            Some("failed") => prompt = Some((Prompt::Failed, "FAILED")),
            _ => {}
        }

        let shown = self.display.show_tasks(board, &scene!(self, now), state, title.map(str::as_bytes), &tasks);
        if shown.is_err() {
            Self::write_literal(board, "DISPLAY ERROR\n");
        } else {
            Self::write_value_line(board, "DISPLAY STATE ", state_label.as_bytes());
        }
        if let Some((prompt, label)) = prompt {
            self.play_prompt(board, prompt, label);
        }
    }

    fn handle_line<B: Board>(&mut self, board: &mut B, line: &[u8]) {
        if line.is_empty() {
            return;
        }
        let Ok(value) = serde_json::from_slice::<Value>(line) else {
            Self::write_literal(board, "ERROR invalid_json\n");
            return;
        };
        // title 可以没有，有就必须是字符串；其余由协议信封本身把关。
        let title_ok = match value.get("title") {
            None => true,
            Some(title) => title.is_string(),
        };
        let parsed = if value.is_object() && title_ok { serde_json::from_value::<Event>(value).ok() } else { None };
        let Some(message) = parsed else {
            Self::write_literal(board, "ERROR invalid_message\n");
            return;
        };
        match message.validate() {
            Ok(()) => {}
            Err(ProtocolError::UnsupportedVersion(_)) => {
                Self::write_literal(board, "ERROR unsupported_version\n");
                return;
            }
            Err(_) => {
                Self::write_literal(board, "ERROR invalid_message\n");
                return;
            }
        }
        debug_assert_eq!(message.version, VERSION);
        let event = message.event.as_str();
        let fields = &message.extra;
        let now = board.now_ms();

        match event {
            // 心跳只用于证明链路存活，不显示也不回显；每 5 秒一次的诊断行会淹没日志。
            // 它顺带捎来 Mac 端的构建标识：设备可能随时重启，一次性的握手会丢。
            "device.heartbeat" => {
                if let Some(build) = string(fields, "build") {
                    self.display.set_daemon_build(board, &scene!(self, now), build.as_bytes());
                }
                // 本地小时数也随心跳来：设备没有时钟，白天黑夜只能听 Mac 端的。
                if let Some(hour) = number(fields, "hour").map(|hour| hour as i32)
                    && hour != self.reported_hour
                {
                    self.reported_hour = hour;
                    self.leisure.set_hour(hour);
                    let line = text!(24, "CLOCK HOUR {}\n", hour % 100);
                    board.write(line.as_bytes());
                }
                // 本地日期也随心跳来：番茄钟的当日记录按它清零。
                if let Some(day) = number(fields, "day")
                    && day > 0.0
                    && self.pomodoro.set_day(day as u32)
                {
                    self.save_tally(board);
                    self.refresh(board);
                }
            }
            // 截图是调试动作，不算 Agent 的动静，也不叫醒休闲。
            "device.screenshot" => {
                let display = &self.display;
                board.with_frame_and_output(&mut |frame, output| {
                    display.dump(frame, &mut |line| {
                        output(line);
                        output(b"\n");
                    });
                });
            }
            // Mac 端刚连上时问一声：模式、固件构建号、音色只在开机或变化时才报，
            // daemon 比设备重启得勤，不问就一直不知道。
            "device.hello" => self.announce_state(board),
            // 眨眼确认与语音包写入都是 App 在操作设备本身，同样不算 Agent 的动静。
            "device.identify" => {
                self.display.identify(now);
                Self::write_literal(board, "IDENTIFY\n");
            }
            // 音量：App 的滑块从这里落到 codec 并存起来；不带 level 只是问一声。
            // 试听走 play_prompt，静音时同样不出声，和别的播报一个规矩。
            "device.volume" => {
                if let Some(level) = number(fields, "level") {
                    let wanted = if level < 0.0 { 0 } else { level as u32 };
                    let level = clamp_volume(wanted);
                    let saved = match board.set_volume(level) {
                        Ok(()) => {
                            self.volume = level;
                            self.save_settings(board)
                        }
                        Err(_) => false,
                    };
                    if !saved {
                        Self::write_literal(board, "VOLUME ERROR\n");
                    }
                }
                self.announce_volume(board);
                if fields.get("preview") == Some(&Value::Bool(true)) {
                    self.play_prompt(board, Prompt::Done, "DONE");
                }
            }
            _ if event.starts_with("voice.") => self.handle_voice_event(board, fields, event),
            // 链路自检：把收到的字符串的长度与 CRC 回给 Mac，查串口是否收错字节。
            "device.echo" => {
                if let Some(data) = string(fields, "data") {
                    let reply = text!(96, "{{\"version\":1,\"event\":\"echo\",\"length\":{},\"crc\":{}}}\n", data.len(), crc32(0, data.as_bytes()));
                    board.write(reply.as_bytes());
                    // 原样回显一行，Mac 端逐字节比对，看串口到底收成了什么。
                    Self::write_value_line(board, "ECHO ", data.as_bytes());
                }
            }
            _ => {
                // Agent 一有动静，小灯灵立刻回来值班。
                self.leisure.note_activity(now);
                if self.display.mode() == Mode::Leisure {
                    self.leisure.tick(now);
                    self.set_mode(board, Mode::Duty);
                }
                self.parse_stats(fields);
                Self::write_value_line(board, "EVENT ", event.as_bytes());
                if let Some(title) = &message.title {
                    Self::write_value_line(board, "TITLE ", title.as_bytes());
                }
                self.show_event(board, fields, event, message.title.as_deref());
            }
        }
    }

    /// 把设备的静态状态整个报一遍：固件构建号、模式、音色、音量。开机与 hello
    /// 都走这里。
    fn announce_state<B: Board>(&self, board: &mut B) {
        Self::write_value_line(board, "DISPLAY READY BUILD ", &self.build);
        Self::write_value_line(board, "MODE ", self.display.mode().name().as_bytes());
        Self::write_value_line(board, "VOICES ", self.voices.current_id().as_bytes());
        self.announce_volume(board);
    }

    /// 语音包写入的回执都是 JSON 行：Mac 端要按序号做停等流控，诊断行不够用。
    fn voice_reply<B: Board>(board: &mut B, event: &str, seq: i64, detail: &str) {
        let line = match event {
            "voice.written" => text!(160, "{{\"version\":1,\"event\":\"voice.written\",\"voice\":\"{}\"}}\n", detail),
            "voice.error" => text!(160, "{{\"version\":1,\"event\":\"voice.error\",\"seq\":{},\"message\":\"{}\"}}\n", seq, detail),
            _ => text!(160, "{{\"version\":1,\"event\":\"{}\",\"seq\":{}}}\n", event, seq),
        };
        board.write(line.as_bytes());
    }

    fn handle_voice_event<B: Board>(&mut self, board: &mut B, fields: &BTreeMap<alloc::string::String, Value>, event: &str) {
        match event {
            "voice.begin" => {
                let Some(size) = number(fields, "size") else {
                    Self::voice_reply(board, "voice.error", -1, VoiceError::InvalidArg.name());
                    return;
                };
                // 分区不在、大小不对就直接回错，不去打断正在播的那一句。
                if let Err(error) = self.voices.check_begin(size as u32) {
                    Self::voice_reply(board, "voice.error", -1, error.name());
                    return;
                }
                // 正在播的那一句可能就读着这块分区：先让播放停下、DMA 冲成静音，
                // 再擦。擦分区要好几秒，这期间主循环停着，DMA 会把缓冲里的东西
                // 一遍遍重放，所以必须先是静音。等待在 poll 里继续。
                board.stop_audio();
                self.pending_voice_begin = Some((size as u32, board.now_ms()));
                self.continue_voice_begin(board);
            }
            "voice.chunk" => {
                let seq = number(fields, "seq");
                let data = string(fields, "data");
                let crc = number(fields, "crc");
                let (Some(seq), Some(data), Some(crc)) = (seq, data, crc) else {
                    Self::voice_reply(board, "voice.error", -1, "invalid chunk");
                    self.voices.abort(board.flash());
                    return;
                };
                match self.voices.chunk(board.flash(), seq as u32, data.as_bytes(), crc as u32) {
                    Ok(()) => Self::voice_reply(board, "voice.ack", seq as i64, ""),
                    Err(error) => {
                        Self::voice_reply(board, "voice.error", seq as i32 as i64, error.name());
                        self.voices.abort(board.flash());
                    }
                }
            }
            "voice.end" => {
                if let Err(error) = self.voices.end(board.flash()) {
                    Self::voice_reply(board, "voice.error", -1, error.name());
                    self.voices.abort(board.flash());
                    return;
                }
                let id = alloc::string::String::from(self.voices.current_id());
                Self::voice_reply(board, "voice.written", -1, &id);
                Self::write_value_line(board, "VOICES ", id.as_bytes());
                // 写完用新音色说一句：桥接上写要好几分钟，人未必守着 App 找试听键；
                // 盒子自己开口就是最直接的"写好了"（2026-09-22 同事写完以为没声音）。
                self.play_prompt(board, Prompt::Done, "DONE");
            }
            _ => Self::voice_reply(board, "voice.error", -1, "unknown voice event"),
        }
    }

    /// voice.begin 的后半：播放停下了（或者等够了）就擦分区、回 voice.ready。
    fn continue_voice_begin<B: Board>(&mut self, board: &mut B) {
        let Some((size, since)) = self.pending_voice_begin else {
            return;
        };
        if board.audio_busy() && (board.now_ms().wrapping_sub(since) as i32) < AUDIO_DRAIN_MS as i32 {
            return;
        }
        self.pending_voice_begin = None;
        match self.voices.begin(board.flash(), size) {
            Ok(()) => Self::voice_reply(board, "voice.ready", -1, ""),
            Err(error) => Self::voice_reply(board, "voice.error", -1, error.name()),
        }
    }
}
