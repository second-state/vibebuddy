//! Speech recognition for voice commands (ADR-0017): a recorded turn, as a WAV file, in; its text out.
//!
//! Behind [`Recognizer`] so the backend can change. In the order they are picked:
//!
//! - The user's own recognizer: any OpenAI-compatible transcription endpoint (`VIBEBUDDY_STT_BASE_URL`,
//!   `VIBEBUDDY_STT_API_KEY`, `VIBEBUDDY_STT_MODEL`).
//! - The voice service in `vibebuddy-cloud`, `POST {base}/v1/voice/transcribe` with the account's session (ADR-0018,
//!   docs/api.md), at the base spoken replies use ([`crate::synthesis::service_base`]): staging outside a release,
//!   none in a release until production opens.
//! - Cloudflare Workers AI's Whisper (`CLOUDFLARE_ACCOUNT_ID`, `CLOUDFLARE_AI_TOKEN`), for development; picked only
//!   by name now that the voice service recognizes.
//!
//! A setting comes from the environment, else from `speech.json` in the config directory (the user's own
//! recognizer, which an App setting will write later: a file of its own, mode 0600, because `PUT /v1/config`
//! replaces the whole config and an older App would wipe a key it doesn't know), else from a `KEY=VALUE` file named
//! by `VIBEBUDDY_STT_ENV_FILE` (development credentials). All are read at start. Credentials are never logged.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use serde_json::{Value, json};
use tracing::{info, warn};

use crate::account::Account;

/// Picks the backend: `service`, `cloudflare`, `openai` or `off`. Unset, the user's own recognizer if set, else the
/// voice service.
pub const BACKEND_VARIABLE: &str = "VIBEBUDDY_STT";
/// A `KEY=VALUE` file the other variables can come from, so credentials stay out of the daemon's environment.
pub const ENV_FILE_VARIABLE: &str = "VIBEBUDDY_STT_ENV_FILE";
/// The language spoken, as an ISO 639-1 code (`zh`, `en`); unset (or `auto`), the recognizer detects it.
pub const LANGUAGE_VARIABLE: &str = "VIBEBUDDY_STT_LANGUAGE";
/// The user's own recognizer, in the config directory: `{"base_url","api_key","model","language"}`.
pub const SETTINGS_FILE: &str = "speech.json";
const CLOUDFLARE_ACCOUNT: &str = "CLOUDFLARE_ACCOUNT_ID";
const CLOUDFLARE_TOKEN: &str = "CLOUDFLARE_AI_TOKEN";
const OWN_BASE_URL: &str = "VIBEBUDDY_STT_BASE_URL";
const OWN_API_KEY: &str = "VIBEBUDDY_STT_API_KEY";
const OWN_MODEL: &str = "VIBEBUDDY_STT_MODEL";
const CLOUDFLARE_MODEL: &str = "@cf/openai/whisper-large-v3-turbo";
/// A 30 s recording is under 1.5 MB of WAV; Workers AI took 3.6 to 4.4 s for 5 s of it.
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

    /// Whether it can be asked now; the voice service needs the person signed in.
    fn ready(&self) -> bool {
        true
    }

    /// The voice service, which a box on the Relay can call itself with a voice token; the user's own recognizer
    /// can't be, so its audio comes over the link (ADR-0018).
    fn is_voice_service(&self) -> bool {
        false
    }
}

/// An HTTP request, built apart from sending it so the tests can check it.
#[derive(Debug)]
pub struct HttpRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Whisper on Cloudflare Workers AI.
pub struct Cloudflare {
    account_id: String,
    token: String,
    language: Option<String>,
}

impl Cloudflare {
    fn request(&self, wav: &[u8]) -> HttpRequest {
        let mut body = json!({"audio": base64::engine::general_purpose::STANDARD.encode(wav)});
        if let Some(language) = &self.language {
            body["language"] = json!(language);
        }
        HttpRequest {
            url: format!("https://api.cloudflare.com/client/v4/accounts/{}/ai/run/{CLOUDFLARE_MODEL}", self.account_id),
            headers: vec![
                ("Authorization".to_owned(), format!("Bearer {}", self.token)),
                ("Content-Type".to_owned(), "application/json".to_owned()),
            ],
            body: body.to_string().into_bytes(),
        }
    }

    fn parse(status: u16, body: &str) -> Result<Transcript, String> {
        let parsed: Value = serde_json::from_str(body).map_err(|_| format!("Workers AI answered {status} with no JSON"))?;
        if let Some(text) = parsed.pointer("/result/text").and_then(Value::as_str) {
            let language = parsed.pointer("/result/transcription_info/language").and_then(Value::as_str);
            return Ok(Transcript { text: text.trim().to_owned(), language: language.map(str::to_owned) });
        }
        let message = parsed.pointer("/errors/0/message").and_then(Value::as_str).unwrap_or("no text in the answer");
        Err(format!("Workers AI answered {status}: {message}"))
    }
}

impl Recognizer for Cloudflare {
    fn recognize(&self, wav: &[u8]) -> Result<Transcript, RecognitionError> {
        let (status, body) = send(self.request(wav))?;
        Ok(Self::parse(status, &body)?)
    }
}

/// The voice service: the WAV as the body, `{"text","language"}` back. It tries Groq, then Workers AI.
pub struct VoiceService {
    base_url: String,
    language: Option<String>,
    /// The account whose session the call carries; None only in tests.
    session: Option<Arc<Account>>,
}

impl VoiceService {
    fn request(&self, wav: &[u8]) -> HttpRequest {
        let query = self.language.as_ref().map(|language| format!("?language={language}")).unwrap_or_default();
        HttpRequest {
            url: format!("{}/v1/voice/transcribe{query}", self.base_url.trim_end_matches('/')),
            headers: [("Content-Type".to_owned(), "audio/wav".to_owned())]
                .into_iter()
                .chain(self.token().map(|token| ("Authorization".to_owned(), format!("Bearer {token}"))))
                .collect(),
            body: wav.to_vec(),
        }
    }

    fn token(&self) -> Option<String> {
        self.session.as_ref().and_then(|session| session.token())
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
}

impl Recognizer for VoiceService {
    fn recognize(&self, wav: &[u8]) -> Result<Transcript, RecognitionError> {
        if self.session.is_some() && self.token().is_none() {
            return Err(signed_out());
        }
        let (status, body) = send(self.request(wav))?;
        if let Some(refusal) = refusal(status, &body, self.session.as_deref()) {
            return Err(refusal);
        }
        Ok(Self::parse(status, &body)?)
    }

    fn ready(&self) -> bool {
        self.session.is_none() || self.token().is_some()
    }

    fn is_voice_service(&self) -> bool {
        true
    }
}

/// The service's refusals the box is told about by name: no session (401; the daemon then forgets it), and the daily
/// allowance or the global cap (429, ADR-0018). The messages are what the box shows.
pub fn refusal(status: u16, body: &str, session: Option<&Account>) -> Option<RecognitionError> {
    match status {
        401 => {
            if let Some(session) = session {
                session.forget();
            }
            Some(signed_out())
        }
        429 => Some(match serde_json::from_str::<Value>(body).ok().and_then(|value| value.get("reason")?.as_str().map(str::to_owned)).as_deref() {
            Some("global_cap") => RecognitionError { reason: "global_cap", message: "Voice is paused for today".to_owned() },
            _ => RecognitionError { reason: "allowance", message: "Today's voice is used up".to_owned() },
        }),
        _ => None,
    }
}

fn signed_out() -> RecognitionError {
    RecognitionError { reason: "signed_out", message: "Sign in on the computer".to_owned() }
}

/// An OpenAI-compatible `POST {base}/audio/transcriptions`: OpenAI itself, or a server the user runs.
pub struct OpenAiCompatible {
    base_url: String,
    api_key: Option<String>,
    model: String,
    language: Option<String>,
}

impl OpenAiCompatible {
    fn request(&self, wav: &[u8], boundary: &str) -> HttpRequest {
        let mut body = Vec::new();
        let mut field = |name: &str, extra: &str, value: &[u8]| {
            body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"{extra}\r\n\r\n").as_bytes());
            body.extend_from_slice(value);
            body.extend_from_slice(b"\r\n");
        };
        field("file", "; filename=\"voice.wav\"\r\nContent-Type: audio/wav", wav);
        field("model", "", self.model.as_bytes());
        field("response_format", "", b"json");
        if let Some(language) = &self.language {
            field("language", "", language.as_bytes());
        }
        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        let mut headers = vec![("Content-Type".to_owned(), format!("multipart/form-data; boundary={boundary}"))];
        if let Some(key) = &self.api_key {
            headers.push(("Authorization".to_owned(), format!("Bearer {key}")));
        }
        HttpRequest { url: format!("{}/audio/transcriptions", self.base_url.trim_end_matches('/')), headers, body }
    }

    fn parse(status: u16, body: &str) -> Result<Transcript, String> {
        let parsed: Value = serde_json::from_str(body).map_err(|_| format!("the recognizer answered {status} with no JSON"))?;
        if (200..300).contains(&status)
            && let Some(text) = parsed.get("text").and_then(Value::as_str)
        {
            let language = parsed.get("language").and_then(Value::as_str).map(str::to_owned);
            return Ok(Transcript { text: text.trim().to_owned(), language });
        }
        let message = parsed.pointer("/error/message").and_then(Value::as_str).unwrap_or("no text in the answer");
        Err(format!("the recognizer answered {status}: {message}"))
    }
}

impl Recognizer for OpenAiCompatible {
    fn recognize(&self, wav: &[u8]) -> Result<Transcript, RecognitionError> {
        let mut nonce = [0u8; 12];
        getrandom::fill(&mut nonce).map_err(|error| error.to_string())?;
        let boundary = format!("vibebuddy-{}", nonce.iter().map(|byte| format!("{byte:02x}")).collect::<String>());
        let (status, body) = send(self.request(wav, &boundary))?;
        Ok(Self::parse(status, &body)?)
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
    // The URL may carry the Cloudflare account id; errors name only the host.
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

/// The recognizer the environment and the settings files configure, if any; the voice service calls with `session`.
pub fn from_env(session: Option<Arc<Account>>) -> Option<Arc<dyn Recognizer>> {
    let own = crate::config::config_dir().map(|dir| read_settings(&dir.join(SETTINGS_FILE))).unwrap_or_default();
    let file = std::env::var_os(ENV_FILE_VARIABLE).map(|path| read_env_file(Path::new(&path))).unwrap_or_default();
    let service = crate::synthesis::service_base(
        std::env::var(crate::synthesis::BASE_URL_VARIABLE).ok(),
        crate::relay::release(),
    );
    configure(
        &|name| {
            std::env::var(name)
                .ok()
                .or_else(|| own.get(name).cloned())
                .or_else(|| file.get(name).cloned())
                .filter(|value| !value.trim().is_empty())
        },
        service,
        session,
    )
}

type Named = (&'static str, Arc<dyn Recognizer>);

/// `service` is the voice service's base URL, if there is one.
fn configure(lookup: &dyn Fn(&str) -> Option<String>, service: Option<String>, session: Option<Arc<Account>>) -> Option<Arc<dyn Recognizer>> {
    let language = lookup(LANGUAGE_VARIABLE).filter(|language| language != "auto");
    let shown = language.clone().unwrap_or_else(|| "detected".to_owned());
    let own = || -> Option<Named> {
        let backend = OpenAiCompatible {
            base_url: lookup(OWN_BASE_URL)?,
            api_key: lookup(OWN_API_KEY),
            model: lookup(OWN_MODEL).unwrap_or_else(|| "whisper-1".to_owned()),
            language: language.clone(),
        };
        Some(("an OpenAI-compatible endpoint", Arc::new(backend)))
    };
    let cloudflare = || -> Option<Named> {
        let backend =
            Cloudflare { account_id: lookup(CLOUDFLARE_ACCOUNT)?, token: lookup(CLOUDFLARE_TOKEN)?, language: language.clone() };
        Some(("Cloudflare Workers AI", Arc::new(backend)))
    };
    let service = || -> Option<Named> {
        Some(("the voice service", Arc::new(VoiceService { base_url: service.clone()?, language: language.clone(), session: session.clone() })))
    };
    let backend = lookup(BACKEND_VARIABLE);
    let chosen = match backend.as_deref() {
        Some("off") => return None,
        Some("service") => service(),
        Some("cloudflare") => cloudflare(),
        Some("openai") => own(),
        Some(other) => {
            warn!(backend = %other, "unknown speech recognizer; expected service, cloudflare, openai or off");
            return None;
        }
        // The user's own recognizer is their choice; Cloudflare is only there for development.
        None => own().or_else(service),
    };
    match (&chosen, backend) {
        (Some((name, _)), _) => info!(language = %shown, "speech recognition: {name}"),
        (None, Some(backend)) => warn!(%backend, "the speech recognizer is missing its settings"),
        (None, None) => {}
    }
    chosen.map(|(_, recognizer)| recognizer)
}

/// The user's own recognizer from `speech.json`, under the names of the variables that override it.
fn read_settings(path: &Path) -> HashMap<String, String> {
    let Ok(text) = std::fs::read_to_string(path) else { return HashMap::new() };
    let Ok(Value::Object(settings)) = serde_json::from_str::<Value>(&text) else {
        warn!(path = %path.display(), "the speech settings are not a JSON object; ignored");
        return HashMap::new();
    };
    settings_to_variables(&settings)
}

fn settings_to_variables(settings: &serde_json::Map<String, Value>) -> HashMap<String, String> {
    [("base_url", OWN_BASE_URL), ("api_key", OWN_API_KEY), ("model", OWN_MODEL), ("language", LANGUAGE_VARIABLE)]
        .into_iter()
        .filter_map(|(field, variable)| Some((variable.to_owned(), settings.get(field)?.as_str()?.to_owned())))
        .collect()
}

/// `KEY=VALUE` lines; `export`, quotes, blank lines and `#` comments are allowed. A missing file is logged.
fn read_env_file(path: &Path) -> HashMap<String, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse_env_file(&text),
        Err(error) => {
            warn!(path = %path.display(), %error, "cannot read the speech recognizer's settings file");
            HashMap::new()
        }
    }
}

fn parse_env_file(text: &str) -> HashMap<String, String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            let (key, value) = line.strip_prefix("export ").unwrap_or(line).split_once('=')?;
            let value = value.trim();
            let value = value
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'))
                .or_else(|| value.strip_prefix('\'').and_then(|value| value.strip_suffix('\'')))
                .unwrap_or(value);
            Some((key.trim().to_owned(), value.to_owned()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |name| pairs.iter().find(|(key, _)| *key == name).map(|(_, value)| (*value).to_owned())
    }

    #[test]
    fn the_cloudflare_request_carries_the_wav_as_base64_and_the_language_only_when_set() {
        let backend = Cloudflare { account_id: "acct".to_owned(), token: "tok".to_owned(), language: Some("zh".to_owned()) };
        let request = backend.request(b"RIFFwav");
        assert_eq!(request.url, "https://api.cloudflare.com/client/v4/accounts/acct/ai/run/@cf/openai/whisper-large-v3-turbo");
        assert!(request.headers.contains(&("Authorization".to_owned(), "Bearer tok".to_owned())));
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body, json!({"audio": "UklGRndhdg==", "language": "zh"}));

        let backend = Cloudflare { language: None, ..backend };
        let body: Value = serde_json::from_slice(&backend.request(b"x").body).unwrap();
        assert!(body.get("language").is_none());
    }

    #[test]
    fn the_cloudflare_answer_gives_its_text_and_language_or_its_error() {
        let heard = Cloudflare::parse(200, r#"{"result":{"text":" 你好 ","transcription_info":{"language":"zh"}},"success":true}"#);
        assert_eq!(heard.unwrap(), Transcript { text: "你好".to_owned(), language: Some("zh".to_owned()) });
        let error = Cloudflare::parse(401, r#"{"success":false,"errors":[{"code":10000,"message":"Authentication error"}]}"#);
        assert_eq!(error.unwrap_err(), "Workers AI answered 401: Authentication error");
        assert!(Cloudflare::parse(502, "<html>").is_err());
    }

    #[test]
    fn the_openai_request_is_multipart_with_file_model_and_language() {
        let backend = OpenAiCompatible {
            base_url: "http://127.0.0.1:8000/v1/".to_owned(),
            api_key: Some("sk-test".to_owned()),
            model: "whisper-large-v3".to_owned(),
            language: Some("zh".to_owned()),
        };
        let request = backend.request(b"RIFF\x00\x01", "BOUNDARY");
        assert_eq!(request.url, "http://127.0.0.1:8000/v1/audio/transcriptions");
        assert!(request.headers.contains(&("Content-Type".to_owned(), "multipart/form-data; boundary=BOUNDARY".to_owned())));
        assert!(request.headers.contains(&("Authorization".to_owned(), "Bearer sk-test".to_owned())));
        let mut expected = b"--BOUNDARY\r\nContent-Disposition: form-data; name=\"file\"; filename=\"voice.wav\"\r\nContent-Type: audio/wav\r\n\r\nRIFF\x00\x01\r\n".to_vec();
        expected.extend_from_slice(b"--BOUNDARY\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\nwhisper-large-v3\r\n");
        expected.extend_from_slice(b"--BOUNDARY\r\nContent-Disposition: form-data; name=\"response_format\"\r\n\r\njson\r\n");
        expected.extend_from_slice(b"--BOUNDARY\r\nContent-Disposition: form-data; name=\"language\"\r\n\r\nzh\r\n");
        expected.extend_from_slice(b"--BOUNDARY--\r\n");
        assert_eq!(String::from_utf8_lossy(&request.body), String::from_utf8_lossy(&expected));

        let keyless = OpenAiCompatible { api_key: None, language: None, ..backend };
        let request = keyless.request(b"", "B");
        assert!(!request.headers.iter().any(|(name, _)| name == "Authorization"));
        assert!(!String::from_utf8_lossy(&request.body).contains("language"));
    }

    #[test]
    fn the_openai_answer_gives_its_text_or_its_error() {
        assert_eq!(OpenAiCompatible::parse(200, r#"{"text":"hello there"}"#).unwrap().text, "hello there");
        let error = OpenAiCompatible::parse(400, r#"{"error":{"message":"bad file"}}"#).unwrap_err();
        assert_eq!(error, "the recognizer answered 400: bad file");
    }

    #[test]
    fn the_backend_is_chosen_by_name_or_by_which_settings_are_there() {
        let service = || Some("https://api-staging.korekore.ai".to_owned());
        assert!(configure(&lookup(&[]), None, None).is_none(), "a release has no voice service yet");
        assert!(configure(&lookup(&[]), service(), None).is_some(), "the voice service by default");
        assert!(configure(&lookup(&[("CLOUDFLARE_ACCOUNT_ID", "a"), ("CLOUDFLARE_AI_TOKEN", "t")]), None, None).is_none(), "Cloudflare only by name");
        assert!(configure(&lookup(&[("VIBEBUDDY_STT", "cloudflare"), ("CLOUDFLARE_ACCOUNT_ID", "a")]), service(), None).is_none(), "a token is needed too");
        assert!(configure(&lookup(&[("VIBEBUDDY_STT", "cloudflare"), ("CLOUDFLARE_ACCOUNT_ID", "a"), ("CLOUDFLARE_AI_TOKEN", "t")]), None, None).is_some());
        assert!(configure(&lookup(&[("VIBEBUDDY_STT_BASE_URL", "http://localhost:8000/v1")]), None, None).is_some());
        assert!(configure(&lookup(&[("VIBEBUDDY_STT", "off")]), service(), None).is_none());
        assert!(configure(&lookup(&[("VIBEBUDDY_STT", "openai")]), service(), None).is_none());
        assert!(configure(&lookup(&[("VIBEBUDDY_STT", "service")]), None, None).is_none());
        assert!(configure(&lookup(&[("VIBEBUDDY_STT", "whisper.cpp")]), service(), None).is_none());
    }

    #[test]
    fn the_users_own_recognizer_wins_over_the_voice_service() {
        let both = configure(&lookup(&[("VIBEBUDDY_STT_BASE_URL", "http://127.0.0.1:9/v1")]), Some("http://127.0.0.1:8/".to_owned()), None).unwrap();
        // Nothing listens on ports 8 or 9, so the error names the endpoint that was chosen.
        let error = both.recognize(b"").unwrap_err();
        assert!(error.message.contains("127.0.0.1:9"), "{error}");
    }

    fn scratch(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("vibebuddy-speech-{tag}-{}-{}", std::process::id(), rand_suffix()))
    }

    fn rand_suffix() -> u64 {
        let mut bytes = [0u8; 8];
        getrandom::fill(&mut bytes).unwrap();
        u64::from_le_bytes(bytes)
    }

    #[test]
    fn the_voice_service_carries_the_session_and_needs_one() {
        let session = Arc::new(crate::account::signed_in_for_tests(&scratch("session"), "tok_abc"));
        let service = VoiceService { base_url: "http://127.0.0.1:9".to_owned(), language: None, session: Some(session.clone()) };
        assert!(service.request(b"").headers.contains(&("Authorization".to_owned(), "Bearer tok_abc".to_owned())));
        assert!(service.ready());

        session.forget();
        assert!(!service.ready());
        // Nothing is sent without a session: port 9 has no listener, and the error would name it otherwise.
        assert_eq!(service.recognize(b"").unwrap_err().reason, "signed_out");
    }

    #[test]
    fn the_services_refusals_are_named_for_the_box() {
        let session = crate::account::signed_in_for_tests(&scratch("refusal"), "tok");
        assert_eq!(refusal(429, r#"{"reason":"allowance"}"#, Some(&session)).unwrap().reason, "allowance");
        assert_eq!(refusal(429, r#"{"reason":"global_cap"}"#, Some(&session)).unwrap().reason, "global_cap");
        assert_eq!(refusal(429, "too many", Some(&session)).unwrap().reason, "allowance");
        assert_eq!(refusal(502, "recognition failed", Some(&session)), None);
        assert!(session.token().is_some(), "only a 401 signs out");
        assert_eq!(refusal(401, r#"{"error":"unauthorized"}"#, Some(&session)).unwrap().reason, "signed_out");
        assert!(session.token().is_none(), "a 401 forgets the session");
    }

    #[test]
    fn the_voice_service_gets_the_wav_as_the_body_and_the_language_in_the_query() {
        let service = VoiceService { base_url: "https://api-staging.korekore.ai/".to_owned(), language: Some("zh".to_owned()), session: None };
        let request = service.request(b"RIFFwav");
        assert_eq!(request.url, "https://api-staging.korekore.ai/v1/voice/transcribe?language=zh");
        assert_eq!(request.headers, [("Content-Type".to_owned(), "audio/wav".to_owned())]);
        assert_eq!(request.body, b"RIFFwav");
        let detected = VoiceService { language: None, ..service };
        assert_eq!(detected.request(b"").url, "https://api-staging.korekore.ai/v1/voice/transcribe");

        let heard = VoiceService::parse(200, r#"{"text":" 你好 ","language":"Chinese"}"#).unwrap();
        assert_eq!(heard, Transcript { text: "你好".to_owned(), language: Some("Chinese".to_owned()) });
        assert_eq!(VoiceService::parse(502, "recognition failed\n").unwrap_err(), "the voice service answered 502: recognition failed");
    }

    #[test]
    fn the_settings_file_maps_onto_the_variables() {
        let settings = json!({"base_url": "https://stt.example/v1", "api_key": "k", "model": "m", "extra": 1});
        let variables = settings_to_variables(settings.as_object().unwrap());
        assert_eq!(variables.get("VIBEBUDDY_STT_BASE_URL").map(String::as_str), Some("https://stt.example/v1"));
        assert_eq!(variables.get("VIBEBUDDY_STT_API_KEY").map(String::as_str), Some("k"));
        assert_eq!(variables.get("VIBEBUDDY_STT_MODEL").map(String::as_str), Some("m"));
        assert_eq!(variables.len(), 3);
    }

    #[test]
    fn the_env_file_takes_plain_quoted_and_exported_lines() {
        let parsed = parse_env_file("# dev keys\nA=1\nexport B=\"two\"\n\nC='three'\nnot a line\n");
        assert_eq!(parsed.get("A").map(String::as_str), Some("1"));
        assert_eq!(parsed.get("B").map(String::as_str), Some("two"));
        assert_eq!(parsed.get("C").map(String::as_str), Some("three"));
        assert_eq!(parsed.len(), 3);
    }
}
