//! Speech recognition for voice commands (ADR-0017): a recorded turn, as a WAV file, in; its text out.
//!
//! There are two kinds of key, and no third (ADR-0024). In the order they are picked:
//!
//! 1. The user's own Groq key, set in the App with their other own keys ([`crate::voice_keys`]). It is read at each
//!    turn, so a key set or cleared takes effect at once. The daemon calls Groq with it directly.
//! 2. When the computer is signed in: our voice service in `vibebuddy-cloud`, `POST {base}/v1/voice/transcribe` with
//!    the account's session (docs/api.md), at the account's base ([`crate::synthesis::service_base`]). The service
//!    calls Token Station with the account's Token Station key, which never comes to the computer.
//! 3. Else the box is told `own_key` (SET VOICE KEY).
//!
//! A 403 `{"reason":"own_key"}` from the service (no voice credit left) is told to the box as `own_key` too, and the
//! turn is not tried again. Keys are never logged.

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tracing::{info, warn};

use crate::account::Account;
use crate::voice_keys::{KeyStatus, Provider, VoiceKeys};

/// Groq's OpenAI-compatible API, for the user's own Groq key.
const GROQ_URL: &str = "https://api.groq.com/openai/v1";
const GROQ_MODEL: &str = "whisper-large-v3-turbo";
/// Whisper often writes Mandarin in traditional characters, which the box's font doesn't have. It takes the prompt as
/// what was said before and follows its style, so a plain sentence in simplified Chinese makes it write simplified,
/// and the English half keeps English as English. Not an instruction: Whisper can't follow one, and may echo the prompt
/// on a near-silent recording.
const SIMPLIFIED_PROMPT: &str = "好的，我们开始吧。OK, let's get started.";
/// A 30 s recording is under 1.5 MB of WAV.
const TIMEOUT: Duration = Duration::from_secs(30);
const MAX_RESPONSE: u64 = 1024 * 1024;

/// What was heard.
#[derive(Debug, Default, PartialEq)]
pub struct Transcript {
    /// Empty when nothing was said.
    pub text: String,
    /// The language the recognizer detected, when it says.
    pub language: Option<String>,
}

/// Why a recording wasn't turned into text: `reason` is what the box is told in `voice.error` (docs/protocol.md).
#[derive(Debug, PartialEq)]
pub struct RecognitionError {
    pub reason: &'static str,
    pub message: String,
}

impl From<String> for RecognitionError {
    fn from(message: String) -> Self {
        Self { reason: "unavailable", message }
    }
}

impl std::fmt::Display for RecognitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.message, self.reason)
    }
}

/// Turns one recording into text. Blocking: it is called on a blocking thread.
pub trait Recognizer: Send + Sync {
    /// `wav` is a whole 16-bit PCM WAV file.
    fn recognize(&self, wav: &[u8]) -> Result<Transcript, RecognitionError>;

    /// Why it can't be asked now, as `voice.available` says it (docs/protocol.md): `own_key` when there is no own
    /// Groq key that works and no signed-in account. None when it can be asked.
    fn refusal(&self) -> Option<&'static str> {
        None
    }
}

/// An HTTP request, built apart from sending it so the tests can check it.
#[derive(Debug)]
pub struct HttpRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Our voice service in `vibebuddy-cloud`, `POST {base}/v1/voice/transcribe` with the account's session: the WAV as
/// the body, `{"text","language"}` back (docs/api.md). The service passes it on to Token Station (ADR-0024).
pub struct VoiceService {
    base_url: String,
    session: Arc<Account>,
}

impl VoiceService {
    fn request(&self, wav: &[u8], token: &str) -> HttpRequest {
        HttpRequest {
            url: format!("{}/v1/voice/transcribe", self.base_url.trim_end_matches('/')),
            headers: vec![("Content-Type".to_owned(), "audio/wav".to_owned()), ("Authorization".to_owned(), format!("Bearer {token}"))],
            body: wav.to_vec(),
        }
    }

    fn parse(status: u16, body: &str) -> Result<Transcript, String> {
        if (200..300).contains(&status)
            && let Ok(parsed) = serde_json::from_str::<Value>(body)
            && let Some(text) = parsed.get("text").and_then(Value::as_str)
        {
            let language = parsed.get("language").and_then(Value::as_str).map(str::to_owned);
            return Ok(Transcript { text: text.trim().to_owned(), language });
        }
        Err(format!("the voice service answered {status}: {}", body.trim().chars().take(200).collect::<String>()))
    }

    /// One call, never repeated: a refusal is the box's to show.
    fn recognize(&self, wav: &[u8], token: &str) -> Result<Transcript, RecognitionError> {
        let (status, body) = send(self.request(wav, token))?;
        if let Some(refusal) = refusal(status, &body, Some(&self.session)) {
            return Err(refusal);
        }
        Ok(Self::parse(status, &body)?)
    }
}

/// The service's refusals the box is told about by name: no session (401; the daemon then forgets it), and no voice
/// for this account, so the user's own key is needed (403 `own_key`: the voice credit is used up, or the service
/// doesn't offer it; ADR-0024). Anything else, a 429 rate limit too, is a plain failure. The messages are what the box
/// shows.
pub fn refusal(status: u16, body: &str, session: Option<&Account>) -> Option<RecognitionError> {
    match status {
        401 => {
            if let Some(session) = session {
                session.forget();
            }
            Some(named("signed_out"))
        }
        403 if reason(body).as_deref() == Some("own_key") => Some(named("own_key")),
        _ => None,
    }
}

fn reason(body: &str) -> Option<String> {
    serde_json::from_str::<Value>(body).ok()?.get("reason")?.as_str().map(str::to_owned)
}

/// A refusal with the text the box shows for it, the same as when it comes in `voice.available`.
fn named(reason: &'static str) -> RecognitionError {
    RecognitionError { reason, message: crate::voice::refusal_message(reason).unwrap_or_default() }
}

/// Groq's OpenAI-compatible `POST {base}/audio/transcriptions`, with the user's own key.
struct Groq<'a> {
    base_url: &'a str,
    key: &'a str,
}

impl Groq<'_> {
    fn request(&self, wav: &[u8], boundary: &str) -> HttpRequest {
        let mut body = Vec::new();
        let mut field = |name: &str, extra: &str, value: &[u8]| {
            body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"{extra}\r\n\r\n").as_bytes());
            body.extend_from_slice(value);
            body.extend_from_slice(b"\r\n");
        };
        field("file", "; filename=\"voice.wav\"\r\nContent-Type: audio/wav", wav);
        field("model", "", GROQ_MODEL.as_bytes());
        field("response_format", "", b"json");
        field("prompt", "", SIMPLIFIED_PROMPT.as_bytes());
        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        let headers = vec![
            ("Content-Type".to_owned(), format!("multipart/form-data; boundary={boundary}")),
            ("Authorization".to_owned(), format!("Bearer {}", self.key)),
        ];
        HttpRequest { url: format!("{}/audio/transcriptions", self.base_url.trim_end_matches('/')), headers, body }
    }

    fn parse(status: u16, body: &str) -> Result<Transcript, String> {
        let parsed: Value = serde_json::from_str(body).map_err(|_| format!("Groq answered {status} with no JSON"))?;
        if (200..300).contains(&status)
            && let Some(text) = parsed.get("text").and_then(Value::as_str)
        {
            let language = parsed.get("language").and_then(Value::as_str).map(str::to_owned);
            return Ok(Transcript { text: text.trim().to_owned(), language });
        }
        let message = parsed.pointer("/error/message").and_then(Value::as_str).unwrap_or("no text in the answer");
        Err(format!("Groq answered {status}: {message}"))
    }

    fn send(&self, wav: &[u8]) -> Result<(u16, String), String> {
        let mut nonce = [0u8; 12];
        getrandom::fill(&mut nonce).map_err(|error| error.to_string())?;
        let boundary = format!("vibebuddy-{}", nonce.iter().map(|byte| format!("{byte:02x}")).collect::<String>());
        send(self.request(wav, &boundary))
    }
}

/// Speech recognition (ADR-0024), in this order: the user's own Groq key; else, when signed in, our voice service;
/// else nothing, and the box is told `own_key`.
pub struct Recognition {
    keys: Arc<VoiceKeys>,
    groq_url: String,
    /// None when the service is switched off (`VIBEBUDDY_TTS_BASE_URL=off`).
    service: Option<VoiceService>,
}

impl Recognition {
    /// The service when it can be asked: there is one, and the account is signed in.
    fn service_token(&self) -> Option<(&VoiceService, String)> {
        let service = self.service.as_ref()?;
        Some((service, service.session.token()?))
    }
}

impl Recognizer for Recognition {
    fn recognize(&self, wav: &[u8]) -> Result<Transcript, RecognitionError> {
        let Some(key) = self.keys.key(Provider::Groq) else {
            let (service, token) = self.service_token().ok_or_else(|| named("own_key"))?;
            return service.recognize(wav, &token);
        };
        let (status, body) = Groq { base_url: &self.groq_url, key: &key }.send(wav)?;
        // A refused key stays refused until the user sets another; the box is told to set one.
        let refused = matches!(status, 401 | 403);
        self.keys.refused(Provider::Groq, refused);
        if refused {
            warn!(status, "Groq refused the user's own key");
            return Err(named("own_key"));
        }
        Ok(Groq::parse(status, &body)?)
    }

    fn refusal(&self) -> Option<&'static str> {
        match self.keys.status().groq {
            KeyStatus { set: true, refused: false } => None,
            KeyStatus { set: true, refused: true } => Some("own_key"),
            _ if self.service_token().is_some() => None,
            _ => Some("own_key"),
        }
    }
}

/// An HTTP client on the system's certificate store that hands back every status as an answer: the recognizers
/// here and the voice service in [`crate::synthesis`] share it.
pub fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .http_status_as_error(false)
        .tls_config(ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build())
        .user_agent(concat!("vibebuddyd/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

fn send(request: HttpRequest) -> Result<(u16, String), String> {
    let mut builder = agent(TIMEOUT).post(&request.url);
    for (name, value) in &request.headers {
        builder = builder.header(name, value);
    }
    // Errors name only the host.
    let host = request.url.split('/').nth(2).unwrap_or_default().to_owned();
    let mut response = builder.send(&request.body[..]).map_err(|error| format!("calling {host}: {error}"))?;
    let status = response.status().as_u16();
    let body = response
        .body_mut()
        .with_config()
        .limit(MAX_RESPONSE)
        .read_to_string()
        .map_err(|error| format!("reading the answer from {host}: {error}"))?;
    Ok((status, body))
}

/// The recognizer: the user's own Groq key in `keys`, else our voice service at the account's base, with its session.
pub fn new(session: Arc<Account>, keys: Arc<VoiceKeys>) -> Arc<dyn Recognizer> {
    let service = session.base().map(|base_url| VoiceService { base_url, session });
    match &service {
        Some(service) => info!(base_url = %service.base_url, "speech recognition: your own Groq key, else the voice service"),
        None => info!("speech recognition: your own Groq key"),
    }
    Arc::new(Recognition { keys, groq_url: GROQ_URL.to_owned(), service })
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::sync::Mutex;

    use super::*;

    /// A fake service on a local port: answers each request in turn with the next `(status, body)`, and records each
    /// request as its head and body.
    fn serve(answers: Vec<(u16, &'static str)>) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let record = seen.clone();
        std::thread::spawn(move || {
            for (status, body) in answers {
                let Ok((stream, _)) = listener.accept() else { return };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let (mut head, mut length) = (String::new(), 0);
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap_or(0);
                    }
                    head.push_str(&line);
                }
                let mut body_in = vec![0; length];
                reader.read_exact(&mut body_in).unwrap();
                record.lock().unwrap().push(format!("{head}\n{}", String::from_utf8_lossy(&body_in)));
                let mut stream = stream;
                let _ = write!(stream, "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            }
        });
        (base, seen)
    }

    /// Nothing listens on port 9: a request sent there fails, and its error names it.
    const NOWHERE: &str = "http://127.0.0.1:9";

    struct Rig {
        recognition: Recognition,
        session: Arc<Account>,
        dir: std::path::PathBuf,
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// The recognizer as the daemon builds it, with Groq at `groq` and our service at `service`; signed in with
    /// `token`, if any; with an own Groq `key`, if any.
    fn rig(tag: &str, groq: &str, service: Option<&str>, token: Option<&str>, key: Option<&str>) -> Rig {
        let dir = crate::voice_keys::tests::scratch(&format!("speech-{tag}"));
        let session = Arc::new(match token {
            Some(token) => crate::account::signed_in_for_tests(&dir, token),
            None => crate::account::Account::new(None, Some(&dir), 7331),
        });
        let keys = Arc::new(VoiceKeys::new(Some(&dir)));
        if let Some(key) = key {
            keys.set(&[(Provider::Groq, Some(key.to_owned()))]).unwrap();
        }
        let service = service.map(|base_url| VoiceService { base_url: base_url.to_owned(), session: session.clone() });
        Rig { recognition: Recognition { keys, groq_url: groq.to_owned(), service }, session, dir }
    }

    #[test]
    fn a_groq_key_goes_to_groq_and_not_to_our_service() {
        let (groq, groq_seen) = serve(vec![(200, r#"{"text":" 几点了 ","language":"zh"}"#)]);
        let rig = rig("groq", &groq, Some(NOWHERE), Some("tok_session"), Some("gsk_test"));
        assert_eq!(rig.recognition.refusal(), None);
        let heard = rig.recognition.recognize(b"RIFFwav").unwrap();
        assert_eq!(heard, Transcript { text: "几点了".to_owned(), language: Some("zh".to_owned()) });
        let request = groq_seen.lock().unwrap()[0].clone();
        assert!(request.starts_with("POST /audio/transcriptions"), "{request}");
        assert!(request.to_ascii_lowercase().contains("authorization: bearer gsk_test"), "{request}");
        assert!(!request.contains("tok_session"), "the session never goes to Groq");
        assert!(request.contains("name=\"model\"\r\n\r\nwhisper-large-v3-turbo\r\n") && request.contains("RIFFwav"), "{request}");
        assert!(request.contains("name=\"prompt\"\r\n\r\n好的，我们开始吧。OK, let's get started.\r\n"), "simplified Chinese is asked for: {request}");
    }

    #[test]
    fn a_refused_groq_key_asks_for_another_and_does_not_fall_back_to_our_service() {
        let (groq, _) = serve(vec![(401, "{}")]);
        let rig = rig("groq-refused", &groq, Some(NOWHERE), Some("tok"), Some("gsk_bad"));
        let error = rig.recognition.recognize(b"RIFF").unwrap_err();
        assert_eq!(error.reason, "own_key");
        assert!(!error.message.contains("gsk_bad"));
        assert!(rig.recognition.keys.status().groq.refused);
        assert_eq!(rig.recognition.refusal(), Some("own_key"), "not our service while the key is refused");
        rig.recognition.keys.set(&[(Provider::Groq, Some("gsk_other".to_owned()))]).unwrap();
        assert_eq!(rig.recognition.refusal(), None);
    }

    #[test]
    fn without_a_groq_key_and_signed_in_it_goes_to_our_service_with_the_session() {
        let (service, seen) = serve(vec![(200, r#"{"text":" 你好 ","language":"chinese"}"#)]);
        let rig = rig("service", NOWHERE, Some(&format!("{service}/")), Some("tok_abc"), None);
        assert_eq!(rig.recognition.refusal(), None);
        let heard = rig.recognition.recognize(b"RIFFwav").unwrap();
        assert_eq!(heard, Transcript { text: "你好".to_owned(), language: Some("chinese".to_owned()) });
        let request = seen.lock().unwrap()[0].clone();
        assert!(request.starts_with("POST /v1/voice/transcribe HTTP/1.1\r\n"), "{request}");
        let lower = request.to_ascii_lowercase();
        assert!(lower.contains("authorization: bearer tok_abc") && lower.contains("content-type: audio/wav"), "{request}");
        assert!(request.ends_with("\n\nRIFFwav"), "the WAV is the body: {request}");
    }

    #[test]
    fn signed_out_with_no_groq_key_the_box_is_told_own_key_and_nothing_is_sent() {
        // Port 9 has no listener: a call would fail as unavailable, not as own_key.
        let rig = rig("signed-out", NOWHERE, Some(NOWHERE), None, None);
        assert_eq!(rig.recognition.refusal(), Some("own_key"));
        let error = rig.recognition.recognize(b"RIFF").unwrap_err();
        assert_eq!(error.reason, "own_key");
        assert_eq!(error.message, "Set a Groq key on the computer to talk to me.");
        // Nor with the service switched off.
        let off = self::rig("service-off", NOWHERE, None, Some("tok"), None);
        assert_eq!(off.recognition.refusal(), Some("own_key"));
        assert_eq!(off.recognition.recognize(b"RIFF").unwrap_err().reason, "own_key");
    }

    #[test]
    fn own_key_from_our_service_is_a_refusal_that_is_not_retried() {
        let (service, seen) = serve(vec![(403, r#"{"reason":"own_key"}"#), (200, r#"{"text":"later"}"#)]);
        let rig = rig("service-own-key", NOWHERE, Some(&service), Some("tok"), None);
        let error = rig.recognition.recognize(b"RIFF").unwrap_err();
        assert_eq!(error.reason, "own_key");
        assert_eq!(seen.lock().unwrap().len(), 1, "asked once");
        assert!(rig.session.token().is_some(), "still signed in");
        // The next command asks again, as its own turn.
        // The answer may name no language.
        assert_eq!(rig.recognition.recognize(b"RIFF").unwrap(), Transcript { text: "later".to_owned(), language: None });
        assert_eq!(seen.lock().unwrap().len(), 2);
    }

    #[test]
    fn a_rate_limit_from_our_service_is_a_plain_failure_and_a_401_signs_out() {
        let (service, _) = serve(vec![(429, "rate limited"), (401, r#"{"error":"unauthorized"}"#)]);
        let rig = rig("service-429", NOWHERE, Some(&service), Some("tok"), None);
        let limited = rig.recognition.recognize(b"RIFF").unwrap_err();
        assert_eq!(limited.reason, "unavailable", "{limited}");
        assert!(limited.message.contains("429"), "{limited}");
        assert_eq!(rig.recognition.refusal(), None, "a rate limit doesn't take voice away");
        assert_eq!(rig.recognition.recognize(b"RIFF").unwrap_err().reason, "signed_out");
        assert!(rig.session.token().is_none(), "a 401 forgets the session");
        assert_eq!(rig.recognition.refusal(), Some("own_key"));
    }

    #[test]
    fn another_403_is_a_plain_failure() {
        assert!(refusal(403, r#"{"error":"forbidden"}"#, None).is_none());
        assert!(refusal(429, r#"{"reason":"allowance"}"#, None).is_none(), "the service no longer has allowances");
        let refused = refusal(403, r#"{"reason":"own_key"}"#, None).unwrap();
        assert_eq!(refused.reason, "own_key");
    }

    #[test]
    fn groq_errors_say_what_groq_said() {
        assert_eq!(Groq::parse(400, r#"{"error":{"message":"bad file"}}"#).unwrap_err(), "Groq answered 400: bad file");
        assert!(Groq::parse(502, "<html>").is_err());
        assert_eq!(VoiceService::parse(502, "recognition failed\n").unwrap_err(), "the voice service answered 502: recognition failed");
    }
}
