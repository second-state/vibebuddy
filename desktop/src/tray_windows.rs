//! The notification area icon on Windows, the counterpart of `tray.rs`: Kore as the icon, and the menu of the Mac
//! app's menu bar icon. It lives on the main thread, whose window loop iced runs; picking an item only sends an event
//! to the app, which does the work.

use std::cell::RefCell;
use std::sync::OnceLock;

use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::i18n::tr;
use crate::korekore::picture::BoxPicture;
use crate::status::{Icon as FaceIcon, MenuState};

#[derive(Clone, Copy, Debug)]
pub enum TrayEvent {
    OpenSettings,
    /// The daemon isn't answering: start it again.
    RestartDaemon,
    CheckForUpdates,
    ReportProblem,
    Quit,
}

struct Tray {
    icon: TrayIcon,
    device: MenuItem,
    mode: MenuItem,
    today: MenuItem,
}

thread_local! {
    static TRAY: RefCell<Option<Tray>> = const { RefCell::new(None) };
}

static EVENTS: OnceLock<UnboundedSender<TrayEvent>> = OnceLock::new();

/// Puts the icon in the notification area; the events from it come out of the receiver. Call it on the main thread.
pub fn create() -> Result<UnboundedReceiver<TrayEvent>, String> {
    let (sender, receiver) = unbounded();
    EVENTS.set(sender).map_err(|_| "the tray icon exists already".to_owned())?;
    let menu_state = MenuState::derive(None);
    let device = MenuItem::new(&menu_state.device_line, true, None);
    let mode = MenuItem::new(&menu_state.mode_line, false, None);
    let today = MenuItem::new(&menu_state.today_line, false, None);
    let settings = MenuItem::new(tr("Settings…", &[]), true, None);
    let updates = MenuItem::new(tr("Check for Updates…", &[]), true, None);
    let problem = MenuItem::new(tr("Report a Problem…", &[]), true, None);
    let quit = MenuItem::new(tr("Quit", &[]), true, None);
    let menu = Menu::new();
    menu.append_items(&[&device, &mode, &today, &PredefinedMenuItem::separator(), &settings, &updates, &problem, &quit])
        .map_err(|error| error.to_string())?;
    let actions: Vec<(MenuId, TrayEvent)> = vec![
        (device.id().clone(), TrayEvent::RestartDaemon),
        (settings.id().clone(), TrayEvent::OpenSettings),
        (updates.id().clone(), TrayEvent::CheckForUpdates),
        (problem.id().clone(), TrayEvent::ReportProblem),
        (quit.id().clone(), TrayEvent::Quit),
    ];
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if let Some((_, action)) = actions.iter().find(|(id, _)| *id == event.id) {
            send(*action);
        }
    }));
    // A left click opens Settings, as a click on the icon does on Linux; the menu is on the right button.
    TrayIconEvent::set_event_handler(Some(|event: TrayIconEvent| {
        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
            send(TrayEvent::OpenSettings);
        }
    }));
    let icon = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip("VibeBuddy")
        .with_icon(icon(&BoxPicture::derive(None))?)
        .build()
        .map_err(|error| error.to_string())?;
    TRAY.with(|tray| *tray.borrow_mut() = Some(Tray { icon, device, mode, today }));
    update(&menu_state, &BoxPicture::derive(None));
    Ok(receiver)
}

fn send(event: TrayEvent) {
    if let Some(events) = EVENTS.get() {
        let _ = events.unbounded_send(event);
    }
}

/// Brings the icon and the menu's lines up to date. Call it on the main thread.
pub fn update(menu: &MenuState, kore: &BoxPicture) {
    TRAY.with(|tray| {
        let tray = tray.borrow();
        let Some(tray) = tray.as_ref() else { return };
        tray.device.set_text(&menu.device_line);
        // The device line restarts the daemon, so it can be picked only while the daemon isn't answering.
        tray.device.set_enabled(menu.icon == FaceIcon::DaemonDown);
        tray.mode.set_text(&menu.mode_line);
        tray.today.set_text(&menu.today_line);
        let _ = tray.icon.set_tooltip(Some(format!("VibeBuddy\n{}", menu.device_line)));
        if let Ok(icon) = icon(kore) {
            let _ = tray.icon.set_icon(Some(icon));
        }
    });
}

/// Kore as RGBA, centred in a square: Windows draws the icon square, and the `sun` dot makes Kore wider than tall.
fn icon(kore: &BoxPicture) -> Result<Icon, String> {
    let (width, height, argb) = crate::korekore::tray_icon(kore, 2);
    let side = width.max(height);
    let (left, top) = ((side - width) / 2, (side - height) / 2);
    let mut rgba = vec![0u8; (side * side * 4) as usize];
    for y in 0..height {
        for x in 0..width {
            let from = ((y * width + x) * 4) as usize;
            let to = (((y + top) * side + x + left) * 4) as usize;
            rgba[to..to + 4].copy_from_slice(&[argb[from + 1], argb[from + 2], argb[from + 3], argb[from]]);
        }
    }
    Icon::from_rgba(rgba, side, side).map_err(|error| error.to_string())
}
