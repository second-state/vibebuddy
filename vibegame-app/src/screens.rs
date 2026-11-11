//! The screens (docs/vibegame.md, "On the VibeGame"): not paired (the code), looking for a computer, on duty, the
//! three full-screen states, no link (gray, eyes closed), Pomodoro and the menu. The layout follows korekore's box
//! screens, made smaller for 240×160.

use qapp::full::{Frame, lang};
use vibegame_core::duty::{State, Task};
use vibegame_core::pomodoro::{Phase, Run};

use crate::app::{App, FPS, MENU, MenuItem, Page};
use crate::art::*;

fn t(en: &'static str, zh: &'static str) -> &'static str {
    lang().pick(en, zh)
}

/// Draws the frame for this moment and picks its palette.
pub fn draw(app: &App, art: &Art, frame: &mut Frame) {
    frame.palette = art.palette;
    frame.clear(INK);
    let tick = app.frames;
    if let Some(code) = app.server.code() {
        pairing(app, frame, code, tick);
    } else if app.page == Page::Pomodoro {
        pomodoro(app, art, frame, tick);
    } else if app.heard.is_none() {
        looking(app, frame, tick);
    } else if !app.link_alive() {
        duty(app, art, frame, tick, false);
        frame.palette = art.gray();
    } else {
        match app.duty.state {
            State::InputRequired if app.picked.is_none() => takeover(app, art, frame, tick),
            State::Done | State::Failed if app.picked.is_none() => takeover(app, art, frame, tick),
            _ => duty(app, art, frame, tick, true),
        }
    }
    if let Some(menu) = app.menu {
        draw_menu(app, frame, menu.at, menu.confirm);
    }
}

/// The robot's face for a calm moment: a blink every few seconds, eyes that move while it works.
fn calm(state: State, tick: u64) -> Mood {
    if tick % (FPS * 4) < 6 {
        return Mood::Blink;
    }
    match state {
        State::Working => Mood::Working([0, 1, 0, -1][(tick / (FPS * 3 / 2) % 4) as usize]),
        _ => Mood::Idle,
    }
}

fn pairing(app: &App, frame: &mut Frame, code: &str, tick: u64) {
    robot(frame, 12, 18, 2, calm(State::Idle, tick));
    let x = 76;
    text(frame, x, 8, t("Pair with a computer", "和电脑配对"), TEXT);
    let how = t("Open VibeBuddy on your computer and pick this VibeGame.", "在电脑上打开 VibeBuddy，选这台掌机，输入下面的配对码。");
    for (n, line) in wrap(how, SCREEN_W - x - 6).iter().enumerate() {
        text(frame, x, 26 + n as i32 * 13, line, MUTED);
    }
    let spaced = format!("{} {}", &code[..3], &code[3..]);
    let k = 3;
    big(frame, x + (SCREEN_W - x - 6 - big_width(&spaced, k)) / 2, 66, &spaced, SUN, k);
    let name = app.server.identity().name();
    text(frame, x, 112, t("Shown on the computer as", "在电脑上叫"), MUTED);
    text(frame, x, 125, &fit(&name, SCREEN_W - x - 6), LAVENDER);
    text_mid(frame, 0, 72, 76, t("PAIRING", "配对"), LAVENDER);
    if !app.server.computers().is_empty() {
        key_hint(frame, 8, 144, "B", t("back", "返回"), MUTED);
    }
}

fn looking(app: &App, frame: &mut Frame, tick: u64) {
    let look = [0, 1, 0, -1][(tick / FPS % 4) as usize];
    robot(frame, 94, 12, 2, Mood::Working(look));
    let dots = ".".repeat((tick / (FPS / 2) % 4) as usize);
    let title = format!("{}{dots}", t("Looking for your computer", "在找你的电脑"));
    text(frame, (SCREEN_W - width(t("Looking for your computer", "在找你的电脑"))) / 2, 66, &title, TEXT);
    let hint = t("Open VibeBuddy on it, on the same Wi-Fi.", "在电脑上打开 VibeBuddy，连同一个 Wi-Fi。");
    for (n, line) in wrap(hint, 220).iter().enumerate() {
        text_mid(frame, 0, SCREEN_W, 82 + n as i32 * 13, line, MUTED);
    }
    let names: Vec<&str> = app.server.computers().iter().map(|c| c.name.as_str()).collect();
    let paired = format!("{}{}", t("Paired: ", "已配对："), names.join(", "));
    text_mid(frame, 0, SCREEN_W, 118, &fit(&paired, 228), LAVENDER);
    key_hint(frame, 8, 144, "X", t("menu", "菜单"), MUTED);
    key_hint(frame, 132, 144, "SELECT", t("Pomodoro", "番茄钟"), MUTED);
}

fn state_word(state: State) -> (&'static str, u8) {
    match state {
        State::Idle => (t("READY", "待命"), LAVENDER),
        State::Working => (t("WORKING", "工作中"), SKY),
        State::InputRequired => (t("NEEDS YOU", "等你"), SUN),
        State::Done => (t("DONE", "完成"), MINT),
        State::Failed => (t("FAILED", "失败"), PEACH),
    }
}

/// "7" from a stats line like "7 DONE".
fn done_today(stats: &[String]) -> Option<u32> {
    let line = stats.iter().find(|line| line.contains("DONE"))?;
    line.split_whitespace().next()?.parse().ok()
}

fn elapsed(seconds: u64) -> String {
    match seconds {
        s if s < 60 => format!("{s}S"),
        s if s < 3600 => format!("{}M", s / 60),
        s => format!("{}H", s / 3600),
    }
}

/// One task card, `w` wide and 34 high.
#[allow(clippy::too_many_arguments)]
fn card(art: &Art, frame: &mut Frame, x: i32, y: i32, w: i32, task: &Task, ground: u8, label: (&str, u8)) {
    frame.rect(x, y, w, 34, ground);
    let mut left = x + 6;
    if let Some(agent) = task.agent {
        art.icon(frame, agent, x + 6, y + 9);
        left = x + 28;
    }
    let pill_w = width(label.0) + 6;
    text(frame, left, y + 4, &fit(&task.title, x + w - left - pill_w - 8), TEXT);
    pill(frame, x + w - 5, y + 4, label.0, label.1);
    let time = elapsed(task.elapsed_s);
    text_right(frame, x + w - 5, y + 19, &time, MUTED);
    if let Some(project) = &task.project {
        text(frame, left, y + 19, &fit(project, x + w - left - width(&time) - 10), MUTED);
    }
}

fn task_label(task: &Task) -> (&'static str, u8) {
    if task.waiting { (t("ASK", "等你"), SUN) } else { (t("RUN", "运行"), SKY) }
}

fn duty(app: &App, art: &Art, frame: &mut Frame, tick: u64, alive: bool) {
    let d = &app.duty;
    // Left: the robot, the state, today.
    let mood = if alive { calm(d.state, tick) } else { Mood::Asleep };
    robot(frame, 11, 12, 2, mood);
    let (word, colour) = if alive { state_word(d.state) } else { (t("NO LINK", "没连上"), MUTED) };
    text_mid(frame, 0, 74, 66, word, colour);
    if let Some(done) = done_today(&d.stats) {
        text_mid(frame, 0, 74, 80, &lang().pick(format!("{done} done today"), format!("今日完成 {done}")), MUTED);
    }
    if !alive {
        text_mid(frame, 0, 74, 94, t("waiting for", "等电脑"), MUTED);
        text_mid(frame, 0, 74, 107, t("the computer", "回来"), MUTED);
    }
    key_hint(frame, 6, 144, "X", t("menu", "菜单"), MUTED);
    // Right: the cards.
    let x = 78;
    let w = SCREEN_W - x - 4;
    text(frame, x, 3, t("ON DUTY", "值班中"), LAVENDER);
    if let Some(name) = app.server.linked() {
        text_right(frame, x + w, 3, &fit(name, w - 60), MUTED);
    }
    if d.tasks.is_empty() {
        let line = match &d.greeting {
            Some(occasion) if occasion.starts_with("greeting") => t("Good to see you!", "你好呀！"),
            _ => t("Nothing running now.", "现在没有任务。"),
        };
        text_mid(frame, x, x + w, 62, line, MUTED);
    }
    for (n, task) in d.tasks.iter().enumerate() {
        let y = 19 + n as i32 * 38;
        card(art, frame, x, y, w, task, CARD, task_label(task));
        if app.picked.map(|(at, _)| at) == Some(n) {
            frame.frame_rect(x - 1, y - 1, w + 2, 36, LAVENDER);
            frame.frame_rect(x - 2, y - 2, w + 4, 38, LAVENDER);
        }
    }
    let more = d.tasks_total.saturating_sub(d.tasks.len());
    if more > 0 {
        text(frame, x, 132, &lang().pick(format!("+{more} more"), format!("还有 {more} 个")), MUTED);
    }
    if alive {
        let hint = if app.picked.is_some() { t("open this one", "打开这个") } else { t("open", "打开") };
        let used = key_hint(frame, x, 146 - 2, "A", hint, MUTED);
        if !d.tasks.is_empty() && app.picked.is_none() {
            key_hint(frame, x + used + 8, 144, "↑↓", t("pick", "选"), MUTED);
        } else if app.picked.is_some() {
            key_hint(frame, x + used + 8, 144, "B", t("back", "返回"), MUTED);
        }
    }
}

/// Needs input, done and failed take the whole screen, as on the box.
fn takeover(app: &App, art: &Art, frame: &mut Frame, tick: u64) {
    let d = &app.duty;
    let (ground, mood, line, small, label) = match d.state {
        State::InputRequired => (SUN, Mood::Waiting, t("Your turn!", "该你了！"), t("an Agent needs you", "有个 Agent 在等你"), (t("ASK", "等你"), SUN)),
        State::Done => (MINT, Mood::Happy, t("All done!", "搞定了！"), t("nice work", "干得漂亮"), (t("DONE", "完成"), MINT)),
        _ => (INK, Mood::Worried, t("Uh-oh.", "出错了。"), t("have a look?", "去看看吧？"), (t("FAIL", "失败"), PEACH)),
    };
    frame.clear(ground);
    let ink = if ground == INK { PEACH } else { INK };
    if d.state == State::Failed {
        for n in 0..3 {
            frame.frame_rect(n, n, SCREEN_W - 2 * n, 160 - 2 * n, PEACH);
        }
    }
    let bob = if d.state == State::Done && (tick / 10).is_multiple_of(2) { -2 } else { 0 };
    robot(frame, 12, 10 + bob, 3, mood);
    big(frame, 96, 20, line, ink, 2);
    text(frame, 98, 50, small, if ground == INK { TEXT } else { INK });
    let task = d.headline.as_ref().or_else(|| d.tasks.iter().find(|task| task.waiting == (d.state == State::InputRequired))).or(d.tasks.first());
    if let Some(task) = task {
        card(art, frame, 8, 92, SCREEN_W - 16, task, if ground == INK { CARD } else { INK }, label);
    }
    let waiting = d.tasks.iter().filter(|task| task.waiting).count();
    if d.state == State::InputRequired && waiting > 1 {
        text(frame, 8, 145, &lang().pick(format!("+{} more waiting", waiting - 1), format!("还有 {} 个在等", waiting - 1)), INK);
    }
    let hint = if d.state == State::Failed { t("open the run", "打开看看") } else { t("take me there", "带我过去") };
    let w = width("A") + 4 + 3 + width(hint);
    if ground == INK {
        key_hint(frame, SCREEN_W - 8 - w, 144, "A", hint, TEXT);
    } else {
        key_hint_on(frame, SCREEN_W - 8 - w, 144, "A", hint, INK, ground);
    }
}

fn pomodoro(app: &App, art: &Art, frame: &mut Frame, tick: u64) {
    let now = app.now();
    let p = &app.pomodoro;
    let (label, colour) = match p.phase {
        Phase::Focus => (t("FOCUS", "专注"), PINK),
        Phase::Break => (t("BREAK", "休息"), MINT),
    };
    text(frame, 12, 8, label, colour);
    let left = p.left(now).div_ceil(1000);
    let clock = format!("{:02}:{:02}", left / 60, left % 60);
    let blink = p.run == Run::Paused && (tick / 30).is_multiple_of(2);
    if !blink {
        big(frame, 12, 26, &clock, TEXT, 4);
    }
    let bar = 120;
    frame.rect(12, 80, bar, 6, FACE);
    let done = bar as u64 * (p.length() - p.left(now)) / p.length();
    frame.rect(12, 80, done as i32, 6, colour);
    let first = match p.run {
        Run::Pending => t("start", "开始"),
        Run::Running => t("pause", "暂停"),
        Run::Paused => t("go on", "继续"),
    };
    key_hint(frame, 12, 96, "START", first, MUTED);
    key_hint(frame, 12, 112, t("HOLD START", "按住 START"), t("give up", "放弃"), MUTED);
    // Right: the robot and today.
    let mood = match p.run {
        Run::Running if p.phase == Phase::Focus => Mood::Working(0),
        _ => calm(State::Idle, tick),
    };
    robot(frame, 164, 6, 2, mood);
    text(frame, 160, 62, t("TODAY", "今天"), MUTED);
    for n in 0..4u32 {
        frame.rect(160 + n as i32 * 12, 78, 8, 8, if n < p.completed { PINK } else { FACE });
    }
    text(frame, 160, 92, &lang().pick(format!("{} FOCUS", p.completed), format!("专注 {} 次", p.completed)), PINK);
    // Bottom: the Agents at a glance.
    if app.heard.is_some() {
        let (word, colour) = if app.link_alive() { state_word(app.duty.state) } else { (t("NO LINK", "没连上"), MUTED) };
        let used = text(frame, 12, 130, word, colour);
        if let Some(task) = app.duty.tasks.first() {
            if let Some(agent) = task.agent {
                art.icon(frame, agent, 12 + used + 6, 128);
            }
            text(frame, 12 + used + 26, 130, &fit(&task.title, SCREEN_W - used - 44), TEXT);
        }
    }
    key_hint(frame, 12, 145, "SELECT", t("on duty", "值班"), MUTED);
}

fn draw_menu(app: &App, frame: &mut Frame, at: usize, confirm: bool) {
    let (x, y, w, h) = (40, 26, 160, 108);
    frame.rect(x - 2, y - 2, w + 4, h + 4, INK);
    frame.rect(x, y, w, h, CARD);
    frame.frame_rect(x, y, w, h, LAVENDER);
    if confirm {
        let n = app.server.computers().len();
        let question = match n {
            1 => t("Forget the paired computer?", "忘掉配对的电脑？").to_owned(),
            n => lang().pick(format!("Forget all {n} computers?"), format!("忘掉全部 {n} 台电脑？")),
        };
        for (row, line) in wrap(&question, w - 16).iter().enumerate() {
            text(frame, x + 8, y + 10 + row as i32 * 13, line, TEXT);
        }
        for (row, line) in wrap(t("It has to pair again to show here.", "要重新配对才能再连上。"), w - 16).iter().enumerate() {
            text(frame, x + 8, y + 42 + row as i32 * 13, line, MUTED);
        }
        key_hint(frame, x + 8, y + h - 20, "A", t("forget", "忘掉"), PEACH);
        key_hint(frame, x + 84, y + h - 20, "B", t("cancel", "取消"), MUTED);
        return;
    }
    text(frame, x + 8, y + 6, t("MENU", "菜单"), LAVENDER);
    for (n, item) in MENU.iter().enumerate() {
        let label = match item {
            MenuItem::Pair => t("Pair a computer", "配对电脑").to_owned(),
            MenuItem::Forget => t("Forget computers", "忘掉所有电脑").to_owned(),
            MenuItem::Mute => format!("{}{}", t("Mute: ", "静音："), if app.muted { t("on", "开") } else { t("off", "关") }),
            MenuItem::Close => t("Close", "关闭").to_owned(),
        };
        let row = y + 22 + n as i32 * 20;
        if n == at {
            frame.rect(x + 4, row - 2, w - 8, 17, LAVENDER);
            text(frame, x + 10, row + 1, &label, INK);
        } else {
            text(frame, x + 10, row + 1, &label, TEXT);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_and_times_read_as_on_the_box() {
        assert_eq!(done_today(&["7 DONE".into(), "3 ASKS".into()]), Some(7));
        assert_eq!(done_today(&[]), None);
        assert_eq!((elapsed(42), elapsed(75), elapsed(7300)), ("42S".into(), "1M".into(), "2H".into()));
    }
}
