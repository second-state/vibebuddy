//! The tray icon, as a StatusNotifierItem: Omarchy's shell, KDE and waybar all host these. The icon is the
//! buddy's face and the menu repeats the Mac app's menu bar menu; picking an item only sends an event to
//! the app, which does the work.

use futures::channel::mpsc::UnboundedSender;
use ksni::menu::StandardItem;
use ksni::{Icon, MenuItem, ToolTip};

use crate::face;
use crate::i18n::tr;
use crate::status::{Icon as FaceIcon, MenuState};

#[derive(Clone, Copy, Debug)]
pub enum TrayEvent {
    OpenSettings,
    Quit,
    /// The shell's tray host is gone (not up yet at login, or restarting) or back; ksni registers again by itself.
    HostGone,
    HostBack,
}

pub struct Tray {
    pub menu: MenuState,
    /// The face is drawn in the theme's text color so it reads on the bar.
    pub color: [u8; 3],
    pub events: UnboundedSender<TrayEvent>,
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "vibebuddy".to_owned()
    }

    fn title(&self) -> String {
        "Vibe Buddy".to_owned()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        let [r, g, b] = self.color;
        // A dimmed, eyes-closed face when the box or the daemon is gone, as on the Mac.
        let (eyes_closed, alpha) = match self.menu.icon {
            FaceIcon::Online => (false, 255),
            FaceIcon::Offline | FaceIcon::DaemonDown => (true, 128),
        };
        [1, 2]
            .into_iter()
            .map(|scale| Icon {
                width: (face::SIZE * scale) as i32,
                height: (face::SIZE * scale) as i32,
                data: face::argb(eyes_closed, [r, g, b, alpha], scale),
            })
            .collect()
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: "Vibe Buddy".to_owned(),
            description: self.menu.device_line.clone(),
            ..Default::default()
        }
    }

    fn watcher_online(&self) {
        let _ = self.events.unbounded_send(TrayEvent::HostBack);
    }

    fn watcher_offline(&self, _reason: ksni::OfflineReason) -> bool {
        let _ = self.events.unbounded_send(TrayEvent::HostGone);
        true
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.events.unbounded_send(TrayEvent::OpenSettings);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let line = |label: &str| {
            StandardItem { label: label.replace('_', "__"), enabled: false, ..Default::default() }.into()
        };
        let action = |label: String, event: TrayEvent| {
            StandardItem {
                label,
                activate: Box::new(move |tray: &mut Self| {
                    let _ = tray.events.unbounded_send(event);
                }),
                ..Default::default()
            }
            .into()
        };
        vec![
            line(&self.menu.device_line),
            line(&self.menu.mode_line),
            line(&self.menu.today_line),
            MenuItem::Separator,
            action(tr("Settings…", &[]), TrayEvent::OpenSettings),
            // systemd keeps the daemon running, so unlike on the Mac, quitting leaves the box online.
            action(tr("Quit", &[]), TrayEvent::Quit),
        ]
    }
}
