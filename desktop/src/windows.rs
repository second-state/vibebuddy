//! What systemd, the desktop portal and the XDG tools do for the app on Linux, done the Windows way. There is no
//! service manager to lean on, so here the app starts the daemon and keeps it running, as the Mac app does: quitting
//! the app takes the box offline.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

/// No console window flashes up for a child.
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// How long a daemon that exited waits before the next start, so one that can't start doesn't spin.
const RESTART_DELAY: Duration = Duration::from_secs(3);
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

fn wide(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain(Some(0)).collect()
}

/// The pipe a later copy opens to ask the running one to show Settings.
pub fn instance_pipe() -> String {
    format!(r"\\.\pipe\vibebuddy-desktop-{}", std::env::var("USERNAME").unwrap_or_default())
}

pub enum Claim {
    First,
    /// Another copy runs and was asked to open Settings; this one should quit.
    HandedOff,
}

/// One running copy per user, as on Linux: a named mutex held for the life of the process says which copy runs.
/// `replacing` is set by a restart: the old copy is still on its way out, so wait for the mutex instead of handing off.
pub fn claim(replacing: bool) -> Claim {
    let name = wide(OsStr::new(&format!(r"Local\vibebuddy-desktop-{}", std::env::var("USERNAME").unwrap_or_default())));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        // SAFETY: `name` is a NUL-terminated UTF-16 string that outlives the call. The handle is never closed: the
        // mutex is this copy's for as long as it runs.
        let handle = unsafe { CreateMutexW(std::ptr::null(), 1, name.as_ptr()) };
        // SAFETY: no other call comes between CreateMutexW and this.
        let taken = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        // No mutex to be had: run anyway rather than not at all.
        if handle.is_null() || !taken {
            return Claim::First;
        }
        // SAFETY: a handle CreateMutexW just returned, not used again.
        unsafe { windows_sys::Win32::Foundation::CloseHandle(handle) };
        if !replacing && std::fs::OpenOptions::new().write(true).open(instance_pipe()).is_ok() {
            return Claim::HandedOff;
        }
        if Instant::now() >= deadline {
            eprintln!("vibebuddy-desktop: another copy is still running");
            return Claim::HandedOff;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Opens a link in the browser, or a folder or file in its usual app.
pub fn open(target: &OsStr) -> Result<(), String> {
    let (verb, target) = (wide(OsStr::new("open")), wide(target));
    // SAFETY: both strings are NUL-terminated and outlive the call; the other pointers may be null.
    let result = unsafe {
        ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), target.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL)
    };
    // Above 32 is success, as the API says.
    if result as usize > 32 { Ok(()) } else { Err(format!("cannot open {}", target_text(&target))) }
}

fn target_text(wide: &[u16]) -> String {
    String::from_utf16_lossy(wide.strip_suffix(&[0]).unwrap_or(wide))
}

/// A toast through Windows PowerShell, under its app id: an app needs an installed shortcut for one of its own. The text
/// goes in through the environment, so nothing in it is read as script.
pub fn notify(title: &str, body: &str) -> Result<(), String> {
    const SCRIPT: &str = "$m = [Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime]; \
        $x = $m::GetTemplateContent([Windows.UI.Notifications.ToastTemplateType]::ToastText02); \
        $t = $x.GetElementsByTagName('text'); \
        [void]$t.Item(0).AppendChild($x.CreateTextNode($env:VIBEBUDDY_TITLE)); \
        [void]$t.Item(1).AppendChild($x.CreateTextNode($env:VIBEBUDDY_BODY)); \
        $id = '{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\WindowsPowerShell\\v1.0\\powershell.exe'; \
        $m::CreateToastNotifier($id).Show([Windows.UI.Notifications.ToastNotification]::new($x))";
    std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
        .env("VIBEBUDDY_TITLE", title)
        .env("VIBEBUDDY_BODY", body)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(drop)
        .map_err(|error| format!("cannot show a notification: {error}"))
}

/// Launch at login is a value under the user's Run key.
pub fn launch_at_login() -> bool {
    reg(&["query", RUN_KEY, "/v", "VibeBuddy"]).is_ok()
}

pub fn set_launch_at_login(enabled: bool) -> Result<(), String> {
    if enabled {
        let exe = std::env::current_exe().map_err(|error| error.to_string())?;
        reg(&["add", RUN_KEY, "/v", "VibeBuddy", "/t", "REG_SZ", "/d", &format!("\"{}\"", exe.display()), "/f"])
    } else {
        reg(&["delete", RUN_KEY, "/v", "VibeBuddy", "/f"])
    }
}

fn reg(args: &[&str]) -> Result<(), String> {
    let status = std::process::Command::new("reg.exe")
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|error| format!("cannot run reg: {error}"))?;
    if status.success() { Ok(()) } else { Err(format!("reg {} failed", args[0])) }
}

/// Where the daemon this app started writes its log, and where the last run's log is kept.
pub fn daemon_log() -> Option<PathBuf> {
    crate::state_dir().map(|dir| dir.join("vibebuddyd.log"))
}

fn previous_daemon_log() -> Option<PathBuf> {
    crate::state_dir().map(|dir| dir.join("vibebuddyd.previous.log"))
}

/// The daemon's process id, while it runs, so it can be stopped for a restart.
static DAEMON: Mutex<Option<u32>> = Mutex::new(None);
static STARTED: OnceLock<()> = OnceLock::new();

/// Starts `vibebuddyd.exe` from beside this app, and again whenever it exits. The daemon watches this process and
/// exits with it (`VIBEBUDDY_PARENT_PID`).
pub fn supervise_daemon() {
    if STARTED.set(()).is_err() {
        return;
    }
    std::thread::spawn(|| {
        let Some(exe) = std::env::current_exe().ok().and_then(|exe| Some(exe.parent()?.join("vibebuddyd.exe"))) else {
            return;
        };
        loop {
            match spawn_daemon(&exe) {
                Ok(mut child) => {
                    *DAEMON.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(child.id());
                    let _ = child.wait();
                    *DAEMON.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
                }
                Err(error) => eprintln!("vibebuddy-desktop: cannot start {}: {error}", exe.display()),
            }
            std::thread::sleep(RESTART_DELAY);
        }
    });
}

fn spawn_daemon(exe: &std::path::Path) -> std::io::Result<std::process::Child> {
    let log = daemon_log().ok_or_else(|| std::io::Error::other("no state directory"))?;
    if let Some(dir) = log.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if let Some(previous) = previous_daemon_log() {
        let _ = std::fs::rename(&log, previous);
    }
    let file = std::fs::File::create(&log)?;
    std::process::Command::new(exe)
        .env("VIBEBUDDY_PARENT_PID", std::process::id().to_string())
        .env("VIBEBUDDY_APP_VERSION", env!("CARGO_PKG_VERSION"))
        .stdout(file.try_clone()?)
        .stderr(file)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
}

/// Stops the daemon; the supervisor starts it again.
pub fn restart_daemon() -> Result<(), String> {
    let Some(pid) = *DAEMON.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) else {
        return Err("the daemon isn't running; it starts again by itself in a moment".to_owned());
    };
    std::process::Command::new("taskkill.exe")
        .args(["/PID", &pid.to_string(), "/F"])
        .stdout(std::process::Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map(drop)
        .map_err(|error| format!("cannot stop the daemon: {error}"))
}
