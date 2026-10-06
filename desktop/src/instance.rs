//! One running copy per user, as on the Mac. The first copy holds an abstract Unix socket named after the user; a
//! later copy (the launcher, a second autostart) connects to it, which asks the first to open Settings, and quits.
//! An abstract socket goes away with its process, so a crash leaves nothing stale behind.

use std::io::ErrorKind;
use std::os::linux::net::SocketAddrExt;
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::{SocketAddr, UnixListener, UnixStream};
use std::time::{Duration, Instant};

pub enum Claim {
    /// This is the running copy; a connection to the listener means "open Settings".
    First(UnixListener),
    /// Another copy runs and was asked to open Settings; this one should quit.
    HandedOff,
    /// No socket to be had: run anyway rather than not at all.
    Unavailable,
}

/// `replacing` is set by a restart: the old copy is still on its way out, so wait for its name instead of handing off.
pub fn claim(replacing: bool) -> Claim {
    let Some(address) = address() else { return Claim::Unavailable };
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match UnixListener::bind_addr(&address) {
            Ok(listener) => return Claim::First(listener),
            Err(error) if error.kind() == ErrorKind::AddrInUse => {
                if !replacing && UnixStream::connect_addr(&address).is_ok() {
                    return Claim::HandedOff;
                }
                if Instant::now() >= deadline {
                    eprintln!("vibebuddy-desktop: another copy is still running");
                    return Claim::HandedOff;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(error) => {
                eprintln!("vibebuddy-desktop: cannot check for a running copy ({error})");
                return Claim::Unavailable;
            }
        }
    }
}

/// Per user, since the abstract namespace is shared by everyone on the machine.
fn address() -> Option<SocketAddr> {
    let uid = std::fs::metadata("/proc/self").ok()?.uid();
    SocketAddr::from_abstract_name(format!("vibebuddy-desktop-{uid}")).ok()
}
