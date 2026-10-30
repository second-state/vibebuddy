//! The box on the Relay (ADR-0012, docs/relay-protocol.md): a WebSocket over TLS to the Relay the
//! computer on the cable last named (`relay.korekore.ai` until one does), kept open whenever the box
//! is on Wi-Fi. This task only moves text frames;
//! firmware-core's `RelayBox`, run by the main loop, decides what they say. The certificate is checked
//! against the roots in `certs/relay-roots.pem` (#110), so the box needs the time first: SNTP from
//! `time.cloudflare.com`.

use alloc::ffi::CString;
use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::CStr;
use core::sync::atomic::{AtomicU32, Ordering};

use embassy_futures::select::{Either, Either4, select, select4};
use embassy_net::dns::DnsQueryType;
use embassy_net::tcp::TcpSocket;
use embassy_net::udp::{PacketMetadata, UdpSocket};
use embassy_net::{IpAddress, Stack};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Instant, Ticker, Timer};
use edge_ws::{FrameHeader, FrameType};
use esp_hal::rng::Rng;
use mbedtls_rs::io::Write as _;
use mbedtls_rs::sys::hook::wall_clock::MbedtlsWallClock;
use mbedtls_rs::{Certificate, ClientSessionConfig, Session, SessionConfig, Tls, X509};
use vibebuddy_firmware_core::relay_box;

const TIME_HOST: &str = "time.cloudflare.com";
const ROOTS: &str = concat!(include_str!("../certs/relay-roots.pem"), "\0");
/// Every 30 s the box sends a WebSocket ping, which keeps NATs open, and the exact text `{"t":"ping"}`,
/// which the Relay counts as a sign of life (it can't see ping frames); silence this long from the Relay
/// means the socket is dead (docs/relay-protocol.md).
const PING_EVERY: Duration = Duration::from_secs(30);
const DEAD_AFTER: Duration = Duration::from_secs(75);
/// Largest frame accepted from the Relay; it caps its own at 8 KiB.
const MAX_FRAME: usize = 8 * 1024 + 16;

/// What the Relay task tells the main loop.
pub enum RelayIn {
    /// A new socket's number and challenge nonce: the main loop answers with the box's signed hello.
    Challenge(u32, String),
    /// Any other message, as JSON text.
    Message(String),
    /// The socket is gone.
    Gone,
    /// A diagnostic line: `RELAY CONNECTED`, `RELAY FAILED <why>`.
    Line(String),
}

pub static IN: Channel<CriticalSectionRawMutex, RelayIn, 16> = Channel::new();
/// Text frames to send, from the main loop, each with the number of the socket it was meant for.
/// Frames for an older socket are dropped: a hello signed for its nonce, or data for its computer, sent
/// first on a new socket fails authentication there (seen on relay-staging, 2026-10-12).
pub static OUT: Channel<CriticalSectionRawMutex, (u32, String), 32> = Channel::new();
/// Slots in [`OUT`] the box's output lines leave free, for what the box tells the Relay itself: that the cable took
/// the box must not wait behind a burst of lines for the computer it takes the box from.
pub const CONTROL_ROOM: usize = 2;
/// The socket frames are for now; 0 when there is none.
static SOCKET: AtomicU32 = AtomicU32::new(0);
static NEXT_SOCKET: AtomicU32 = AtomicU32::new(1);

/// The number to queue a frame with: the socket that is up now.
pub fn current_socket() -> u32 {
    SOCKET.load(Ordering::Relaxed)
}
/// The box's id, which names its place on the Relay; signalled once at boot.
pub static BOX_ID: Signal<CriticalSectionRawMutex, String> = Signal::new();
/// The Relay's URL, `wss://host`: signalled at boot and whenever the computer on the cable names
/// another, which drops the socket and connects to the new one.
pub static URL: Signal<CriticalSectionRawMutex, String> = Signal::new();
/// The box is about to restart: close the socket with 1000 and stay off.
pub static STOP: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Unix seconds at `Instant` zero, once SNTP has answered; 0 until then. 32 bits last until 2106, and
/// Xtensa has no 64-bit atomics.
static UNIX_AT_BOOT: AtomicU32 = AtomicU32::new(0);

/// mbedtls's clock for certificate dates: SNTP's time carried forward by the monotonic clock.
pub struct WallClock;

impl MbedtlsWallClock for WallClock {
    fn instant(&self) -> Option<mbedtls_rs::sys::tm> {
        let base = UNIX_AT_BOOT.load(Ordering::Relaxed);
        (base != 0).then(|| civil(u64::from(base) + Instant::now().as_secs()))
    }
}

/// Unix seconds to calendar time (UTC), by Howard Hinnant's days-from-civil inverse.
fn civil(unix: u64) -> mbedtls_rs::sys::tm {
    let days = (unix / 86_400) as i64;
    let seconds = (unix % 86_400) as i32;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 { month_index + 3 } else { month_index - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    // SAFETY: tm is plain old data; every field mbedtls reads is set below.
    let mut tm: mbedtls_rs::sys::tm = unsafe { core::mem::zeroed() };
    tm.tm_sec = seconds % 60;
    tm.tm_min = seconds / 60 % 60;
    tm.tm_hour = seconds / 3600;
    tm.tm_mday = day as _;
    tm.tm_mon = (month - 1) as _;
    tm.tm_year = (year - 1900) as _;
    tm
}

/// The Relay refused this firmware's protocol version: stop trying until it is updated.
const UPGRADE: &str = "update needed";
/// The computer on the cable named another Relay.
const MOVED: &str = "moved";
/// The box is restarting.
const STOPPED: &str = "stopped";

fn line(text: String) {
    let _ = IN.try_send(RelayIn::Line(text));
}

#[embassy_executor::task]
pub async fn task(stack: Stack<'static>, tls: Tls<'static>) {
    let box_id = BOX_ID.wait().await;
    let mut url = URL.wait().await;
    let mut backoff = Duration::from_secs(2);
    // Not stack.wait_config_up(): that waker belongs to net::address_task (see net::ADDRESS).
    let mut address = crate::net::ADDRESS.receiver().expect("Relay address reader");
    loop {
        if let Some(next) = URL.try_take() {
            url = next;
            backoff = Duration::from_secs(2);
        }
        // firmware-core only keeps a URL it can use.
        let host = relay_box::url_host(&url).unwrap_or_default();
        address.get_and(Option::is_some).await;
        if UNIX_AT_BOOT.load(Ordering::Relaxed) == 0 {
            match sntp(stack).await {
                Some(unix) => {
                    UNIX_AT_BOOT.store((unix - Instant::now().as_secs()) as u32, Ordering::Relaxed);
                    line(alloc::format!("CLOCK UNIX {unix}"));
                }
                None => {
                    line(String::from("RELAY FAILED time"));
                    Timer::after(Duration::from_secs(10)).await;
                    continue;
                }
            }
        }
        let started = Instant::now();
        SOCKET.store(NEXT_SOCKET.fetch_add(1, Ordering::Relaxed), Ordering::Relaxed);
        let result = session(stack, &tls, host, &box_id).await;
        SOCKET.store(0, Ordering::Relaxed);
        // Waits for room rather than drop it: a main loop that never hears the socket went keeps its session.
        IN.send(RelayIn::Gone).await;
        match result {
            Err(MOVED) => continue,
            Err(STOPPED) => return,
            // Only another firmware, or another Relay, can help.
            Err(UPGRADE) => {
                line(alloc::format!("RELAY FAILED {UPGRADE}"));
                URL.signal(URL.wait().await);
                continue;
            }
            Err(why) => line(alloc::format!("RELAY FAILED {why}")),
            Ok(()) => {}
        }
        // A socket that lasted a while was fine; start the backoff over.
        if started.elapsed() > Duration::from_secs(60) {
            backoff = Duration::from_secs(2);
        }
        Timer::after(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(60));
    }
}

/// One SNTP query: Unix seconds, or None.
async fn sntp(stack: Stack<'static>) -> Option<u64> {
    let address = stack.dns_query(TIME_HOST, DnsQueryType::A).await.ok()?.first().copied()?;
    let mut rx_meta = [PacketMetadata::EMPTY; 2];
    let mut tx_meta = [PacketMetadata::EMPTY; 2];
    let mut rx = [0u8; 64];
    let mut tx = [0u8; 64];
    let mut socket = UdpSocket::new(stack, &mut rx_meta, &mut rx, &mut tx_meta, &mut tx);
    socket.bind(0).ok()?;
    let mut request = [0u8; 48];
    request[0] = 0x23; // version 4, client
    socket.send_to(&request, (address, 123)).await.ok()?;
    let mut reply = [0u8; 48];
    let received = embassy_time::with_timeout(Duration::from_secs(5), socket.recv_from(&mut reply)).await.ok()?.ok()?;
    if received.0 < 48 {
        return None;
    }
    let seconds_since_1900 = u64::from(u32::from_be_bytes([reply[40], reply[41], reply[42], reply[43]]));
    seconds_since_1900.checked_sub(2_208_988_800).filter(|&unix| unix > 1_700_000_000)
}

async fn session(stack: Stack<'static>, tls: &Tls<'static>, host: &str, box_id: &str) -> Result<(), &'static str> {
    let address = match stack.dns_query(host, DnsQueryType::A).await.ok().and_then(|found| found.first().copied()) {
        Some(IpAddress::Ipv4(address)) => address,
        _ => return Err("dns"),
    };
    // The receive window bounds a firmware update through the Relay to one window per round trip (ADR-0016): at
    // about 0.3 s to the Relay's edge, 32 KB held a box to about 100 KB/s of sealed lines, measured. Without window
    // scaling a TCP window stops just short of 64 KB. On the heap, which is PSRAM first.
    let mut rx = alloc::vec![0u8; 63 * 1024];
    let mut tx = alloc::vec![0u8; 4096];
    let mut socket = TcpSocket::new(stack, &mut rx, &mut tx);
    socket.set_timeout(Some(DEAD_AFTER));
    socket.connect((IpAddress::Ipv4(address), 443)).await.map_err(|_| "connect")?;
    let roots = CStr::from_bytes_with_nul(ROOTS.as_bytes()).map_err(|_| "roots")?;
    let server_name = CString::new(host).map_err(|_| "host")?;
    let config = ClientSessionConfig {
        ca_chain: Some(Certificate::new(X509::PEM(roots)).map_err(|_| "roots")?),
        server_name: Some(&server_name),
        ..ClientSessionConfig::new()
    };
    let mut session = Session::new(tls.reference(), socket, &SessionConfig::Client(config)).map_err(|_| "tls")?;
    session.connect().await.map_err(|_| "tls handshake")?;

    let mut key = [0u8; 16];
    Rng::new().read(&mut key);
    let request = alloc::format!(
        "GET /v1/boxes/{box_id} HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {}\r\nSec-WebSocket-Version: 13\r\n\r\n",
        base64(&key)
    );
    session.write_all(request.as_bytes()).await.map_err(|_| "upgrade")?;
    let mut input: Vec<u8> = Vec::with_capacity(2048);
    let mut chunk = alloc::vec![0u8; 1024];
    // The response headers, up to the blank line; anything after it is already WebSocket.
    let header_end = loop {
        let count = session.read(&mut chunk).await.map_err(|_| "upgrade")?;
        if count == 0 {
            return Err("upgrade");
        }
        input.extend_from_slice(&chunk[..count]);
        if let Some(end) = input.windows(4).position(|window| window == b"\r\n\r\n") {
            break end + 4;
        }
        if input.len() > 4096 {
            return Err("upgrade");
        }
    };
    if !input.starts_with(b"HTTP/1.1 101") {
        return Err(if input.starts_with(b"HTTP/1.1 429") { "rate limited" } else { "upgrade refused" });
    }
    input.drain(..header_end);
    line(String::from("RELAY CONNECTED"));

    let mut ping = Ticker::every(PING_EVERY);
    let mut heard = Instant::now();
    loop {
        // Frames already read, whole ones first.
        while let Some((header, length)) = whole_frame(&input)? {
            let payload_end = length + header.payload_len as usize;
            let payload = input[length..payload_end].to_vec();
            input.drain(..payload_end);
            match header.frame_type {
                FrameType::Text(false) => {
                    let text = String::from_utf8(payload).map_err(|_| "bad text")?;
                    let value: serde_json::Value = serde_json::from_str(&text).map_err(|_| "bad json")?;
                    if value["t"] == "challenge" {
                        let nonce = value["nonce"].as_str().unwrap_or_default();
                        IN.send(RelayIn::Challenge(current_socket(), String::from(nonce))).await;
                    } else {
                        IN.send(RelayIn::Message(text)).await;
                    }
                }
                FrameType::Ping => send_frame(&mut session, FrameType::Pong, &payload).await?,
                // 4005: the Relay no longer speaks this protocol version; only a firmware update helps.
                FrameType::Close if payload.get(..2) == Some(&4005u16.to_be_bytes()) => return Err(UPGRADE),
                FrameType::Close => return Err("closed by the relay"),
                _ => {}
            }
        }
        match select4(session.read(&mut chunk), OUT.receive(), ping.next(), select(URL.wait(), STOP.wait())).await {
            Either4::First(Ok(0)) | Either4::First(Err(_)) => return Err("closed"),
            Either4::First(Ok(count)) => {
                heard = Instant::now();
                input.extend_from_slice(&chunk[..count]);
            }
            Either4::Second((socket_number, text)) if socket_number == current_socket() => {
                send_frame(&mut session, FrameType::Text(false), text.as_bytes()).await?;
            }
            Either4::Second(_) => {}
            Either4::Third(()) => {
                if heard.elapsed() >= DEAD_AFTER {
                    return Err("silent");
                }
                send_frame(&mut session, FrameType::Ping, &[]).await?;
                send_frame(&mut session, FrameType::Text(false), relay_box::PING.as_bytes()).await?;
            }
            // The task picks the new URL up as it connects again.
            Either4::Fourth(Either::Second(())) => {
                let _ = send_frame(&mut session, FrameType::Close, &1000u16.to_be_bytes()).await;
                return Err(STOPPED);
            }
            Either4::Fourth(Either::First(next)) => {
                URL.signal(next);
                let _ = send_frame(&mut session, FrameType::Close, &1000u16.to_be_bytes()).await;
                return Err(MOVED);
            }
        }
    }
}

/// The header of the first frame in `input` and its length, once the whole frame is there.
fn whole_frame(input: &[u8]) -> Result<Option<(FrameHeader, usize)>, &'static str> {
    match FrameHeader::deserialize(input) {
        Ok((header, length)) => {
            if header.payload_len as usize > MAX_FRAME {
                return Err("frame too big");
            }
            Ok((input.len() >= length + header.payload_len as usize).then_some((header, length)))
        }
        Err(edge_ws::Error::Incomplete(_)) => Ok(None),
        Err(_) => Err("bad frame"),
    }
}

/// A client frame, masked as RFC 6455 requires.
async fn send_frame<S: mbedtls_rs::io::Write>(session: &mut S, frame_type: FrameType, payload: &[u8]) -> Result<(), &'static str> {
    let header = FrameHeader { frame_type, payload_len: payload.len() as u64, mask_key: Some(Rng::new().random()) };
    let mut buffer = alloc::vec![0u8; FrameHeader::MAX_LEN + payload.len()];
    let length = header.serialize(&mut buffer).map_err(|_| "frame")?;
    buffer[length..length + payload.len()].copy_from_slice(payload);
    header.mask(&mut buffer[length..length + payload.len()], 0);
    session.write_all(&buffer[..length + payload.len()]).await.map_err(|_| "write")?;
    session.flush().await.map_err(|_| "write")
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let bits = chunk.iter().enumerate().fold(0u32, |acc, (index, &byte)| acc | u32::from(byte) << (16 - 8 * index));
        for index in 0..4 {
            out.push(if index <= chunk.len() { ALPHABET[(bits >> (18 - 6 * index) & 63) as usize] as char } else { '=' });
        }
    }
    out
}
