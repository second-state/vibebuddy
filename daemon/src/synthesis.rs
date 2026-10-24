//! Speech synthesis for spoken replies (ADR-0017): a reply's text in, an MP3 stream out, piece by piece as the
//! service makes it.
//!
//! Behind [`Synthesizer`] so the backend can change. A voice from a provider the user has their own key for
//! ([`crate::voice_keys`]) goes straight to that provider, ElevenLabs or Volcano Engine, and never to the voice
//! service; a provider refusing the key leaves that reply silent rather than spending the service's allowance.
//! Every other reply goes to the voice service in `vibebuddy-cloud`,
//! `POST {base}/v1/voice/speak` with `{"text","voice"}` and the account's session (ADR-0018, docs/api.md), answering a chunked `audio/mpeg` stream (MPEG-2 Layer III,
//! 24 kHz mono, 48 kbps) that starts as soon as there is audio. Builds other than releases use staging; a release
//! has none until the production service is deployed. `VIBEBUDDY_TTS_BASE_URL` sets the base by hand, or `off`;
//! speech recognition ([`crate::speech`]) goes to the same service.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use serde_json::json;
use tracing::{info, warn};

use crate::account::Account;
use crate::voice_keys::{Provider, VoiceKeys};

/// The voice service's base URL, or `off`. Unset: staging, except in a release.
pub const BASE_URL_VARIABLE: &str = "VIBEBUDDY_TTS_BASE_URL";
pub const STAGING_URL: &str = "https://api-staging.korekore.ai";
/// The most text the service takes in one request, in UTF-16 code units (JavaScript's `length`).
pub const MAX_TEXT: usize = 1000;
/// A request gets this long for its whole answer; 1000 characters is about a minute of speech, made faster than
/// it plays.
pub(crate) const TIMEOUT: Duration = Duration::from_secs(120);
/// No reply needs more: ten minutes at 48 kbps.
const MAX_AUDIO: usize = 4 * 1024 * 1024;

/// Turns one reply into speech. Blocking: it is called on a blocking thread.
pub trait Synthesizer: Send + Sync {
    /// Calls `on_audio` with each piece of the MP3 as it arrives (not on frame boundaries); `on_audio` returns false
    /// to stop. An error after some audio means the stream was cut short.
    fn synthesize(&self, text: &str, voice: &str, on_audio: &mut dyn FnMut(&[u8]) -> bool) -> Result<(), String>;

    /// Whether it can speak in `voice` at all; when not, the reply stays text and nothing is tried.
    fn speaks(&self, _voice: &str) -> bool {
        true
    }
}

/// The voice service.
pub struct VoiceService {
    base_url: String,
    /// The account whose session the call carries; None only in tests.
    session: Option<Arc<Account>>,
}

impl VoiceService {
    fn url(&self) -> String {
        format!("{}/v1/voice/speak", self.base_url.trim_end_matches('/'))
    }

    fn speak(&self, text: &str, voice: &str, on_audio: &mut dyn FnMut(&[u8]) -> bool) -> Result<bool, String> {
        let url = self.url();
        let host = url.split('/').nth(2).unwrap_or_default().to_owned();
        let body = json!({"text": text, "voice": voice}).to_string();
        let token = self.session.as_ref().map(|session| session.token().ok_or("not signed in")).transpose()?;
        // One agent for every reply, so its connection to the service is reused: a fresh TLS handshake cost about
        // half a second a reply.
        static AGENT: std::sync::OnceLock<ureq::Agent> = std::sync::OnceLock::new();
        let mut request = AGENT.get_or_init(|| crate::speech::agent(TIMEOUT)).post(&url).header("Content-Type", "application/json");
        if let Some(token) = token {
            request = request.header("Authorization", &format!("Bearer {token}"));
        }
        let mut response = request
            .send(body.as_bytes())
            .map_err(|error| format!("calling {host}: {error}"))?;
        let status = response.status().as_u16();
        if status != 200 {
            let answer = response.body_mut().with_config().limit(1024).read_to_string().unwrap_or_default();
            if let Some(refusal) = crate::speech::refusal(status, &answer, self.session.as_deref()) {
                return Err(refusal.to_string());
            }
            return Err(format!("the voice service answered {status}: {}", answer.trim()));
        }
        forward(response.into_body().into_reader(), &host, on_audio)
    }
}

/// Hands on an MP3 body as it arrives; false when `on_audio` stopped it.
fn forward(mut reader: impl Read, host: &str, on_audio: &mut dyn FnMut(&[u8]) -> bool) -> Result<bool, String> {
    let mut buffer = [0u8; 4096];
    let mut total = 0;
    loop {
        // A connection cut before the end of the chunked body is an error here, not an end.
        let read = reader.read(&mut buffer).map_err(|error| format!("reading audio from {host}: {error}"))?;
        if read == 0 {
            return Ok(true);
        }
        total += read;
        if total > MAX_AUDIO {
            return Err(format!("{host} sent more audio than any reply needs"));
        }
        if !on_audio(&buffer[..read]) {
            return Ok(false);
        }
    }
}

impl Synthesizer for VoiceService {
    fn synthesize(&self, text: &str, voice: &str, on_audio: &mut dyn FnMut(&[u8]) -> bool) -> Result<(), String> {
        // MP3 frames stand alone, so the pieces of a long reply play as one stream.
        for piece in split(text, MAX_TEXT) {
            if !self.speak(piece, voice, on_audio)? {
                break;
            }
        }
        Ok(())
    }
}

pub const ELEVENLABS_URL: &str = "https://api.elevenlabs.io";
/// Flash v2.5 and the format the voice service asks for: 24 kHz mono MP3 at 48 kbps, what the box plays.
pub(crate) const ELEVENLABS_MODEL: &str = "eleven_flash_v2_5";
const ELEVENLABS_FORMAT: &str = "mp3_24000_48";
/// Volcano Engine's V3 one-way stream: back-to-back JSON objects, each with a piece of audio as base64 in `data`,
/// until `code` 20000000.
pub const VOLC_URL: &str = "https://openspeech.bytedance.com/api/v3/tts/unidirectional";
/// The 1.0 voices, as the service uses; [`voice_for`] gives no Character a 2.0 one.
pub(crate) const VOLC_RESOURCE_ID: &str = "seed-tts-1.0";
const VOLC_DONE: i64 = 20_000_000;

/// Spoken replies: with the user's own key for the voice's provider, from that provider; else from the voice
/// service, when there is one.
pub struct Synthesis {
    service: Option<VoiceService>,
    keys: Arc<VoiceKeys>,
    /// The providers' base URLs; others only in tests.
    elevenlabs_url: String,
    volc_url: String,
}

impl Synthesis {
    /// One piece of a reply from `provider`, with the user's own key; false when `on_audio` stopped it.
    fn own(&self, provider: Provider, voice: &str, key: &str, text: &str, on_audio: &mut dyn FnMut(&[u8]) -> bool) -> Result<bool, String> {
        static AGENT: std::sync::OnceLock<ureq::Agent> = std::sync::OnceLock::new();
        let agent = AGENT.get_or_init(|| crate::speech::agent(TIMEOUT));
        let (name, request) = match provider {
            Provider::ElevenLabs => {
                let url = format!("{}/v1/text-to-speech/{voice}/stream?output_format={ELEVENLABS_FORMAT}", self.elevenlabs_url);
                let request = agent.post(&url).header("xi-api-key", key).header("Accept", "audio/mpeg");
                ("ElevenLabs", request.header("Content-Type", "application/json").send(json!({"text": text, "model_id": ELEVENLABS_MODEL}).to_string().as_bytes()))
            }
            Provider::Volc => {
                let body = json!({
                    "user": {"uid": "vibebuddy"},
                    "req_params": {"text": text, "speaker": voice, "audio_params": {"format": "mp3", "sample_rate": 24_000, "bit_rate": 48_000}},
                });
                let request = agent
                    .post(&self.volc_url)
                    .header("X-Api-Key", key)
                    .header("X-Api-Resource-Id", VOLC_RESOURCE_ID)
                    .header("X-Api-Request-Id", &request_id()?)
                    .header("Content-Type", "application/json");
                ("Volcano Engine", request.send(body.to_string().as_bytes()))
            }
        };
        let mut response = request.map_err(|error| format!("calling {name}: {error}"))?;
        let status = response.status().as_u16();
        if matches!(status, 401 | 403) {
            self.keys.refused(provider, true);
            return Err(format!("{name} refused your own key ({status}); the reply stays text"));
        }
        if status != 200 {
            let answer = response.body_mut().with_config().limit(1024).read_to_string().unwrap_or_default();
            return Err(format!("{name} answered {status}: {}", answer.trim().chars().take(200).collect::<String>()));
        }
        self.keys.refused(provider, false);
        let reader = response.into_body().into_reader();
        match provider {
            Provider::ElevenLabs => forward(reader, name, on_audio),
            Provider::Volc => forward_volc(reader, on_audio),
        }
    }
}

impl Synthesizer for Synthesis {
    fn synthesize(&self, text: &str, voice: &str, on_audio: &mut dyn FnMut(&[u8]) -> bool) -> Result<(), String> {
        let own = Provider::of(voice).and_then(|(provider, id)| Some((provider, id, self.keys.key(provider)?)));
        let Some((provider, id, key)) = own else {
            return self.service.as_ref().ok_or("no voice service")?.synthesize(text, voice, on_audio);
        };
        for piece in split(text, MAX_TEXT) {
            if !self.own(provider, id, &key, piece, on_audio)? {
                break;
            }
        }
        Ok(())
    }

    fn speaks(&self, voice: &str) -> bool {
        self.service.is_some() || Provider::of(voice).is_some_and(|(provider, _)| self.keys.key(provider).is_some())
    }
}

/// Volcano Engine's stream of JSON objects, as MP3. Its MP3 comes at 64 kbps whatever `bit_rate` asks, which the
/// box decodes as well as 48, behind an ID3 tag, which is left out so the box gets frames only.
fn forward_volc(reader: impl Read, on_audio: &mut dyn FnMut(&[u8]) -> bool) -> Result<bool, String> {
    let mut tag = Id3::default();
    read_volc(reader, &mut |audio| {
        let audio = tag.strip(audio);
        audio.is_empty() || on_audio(&audio)
    })
}

/// Hands on the audio of each object in Volcano Engine's stream, until the last; false when `on_audio` stopped it.
pub(crate) fn read_volc(reader: impl Read, on_audio: &mut dyn FnMut(&[u8]) -> bool) -> Result<bool, String> {
    let mut total = 0;
    for message in serde_json::Deserializer::from_reader(std::io::BufReader::new(reader)).into_iter::<serde_json::Value>() {
        let message = message.map_err(|error| format!("reading audio from Volcano Engine: {error}"))?;
        match message.get("code").and_then(serde_json::Value::as_i64) {
            Some(VOLC_DONE) => return Ok(true),
            Some(0) => {}
            code => {
                let text = message.get("message").and_then(serde_json::Value::as_str).unwrap_or_default();
                return Err(format!("Volcano Engine: error {}: {text}", code.unwrap_or_default()));
            }
        }
        let Some(data) = message.get("data").and_then(serde_json::Value::as_str).filter(|data| !data.is_empty()) else { continue };
        let audio = base64::engine::general_purpose::STANDARD.decode(data).map_err(|_| "Volcano Engine sent audio that isn't base64")?;
        total += audio.len();
        if total > MAX_AUDIO {
            return Err("Volcano Engine sent more audio than any reply needs".to_owned());
        }
        if !on_audio(&audio) {
            return Ok(false);
        }
    }
    // The connection closed before the last object: cut short.
    Err("Volcano Engine's stream ended early".to_owned())
}

/// Drops an ID3v2 tag at the start of an MP3 stream, however its pieces cut it.
#[derive(Default)]
struct Id3 {
    /// The stream's first bytes, until there are enough to tell whether a tag starts it.
    head: Vec<u8>,
    decided: bool,
    /// Tag bytes still to drop.
    skip: usize,
}

impl Id3 {
    fn strip(&mut self, piece: &[u8]) -> Vec<u8> {
        let mut bytes = if self.decided {
            piece.to_vec()
        } else {
            self.head.extend_from_slice(piece);
            if self.head.len() < 10 {
                return Vec::new();
            }
            self.decided = true;
            if self.head.starts_with(b"ID3") {
                // A syncsafe size (7 bits a byte) after the 10-byte header, and a 10-byte footer if flagged.
                let size = self.head[6..10].iter().fold(0, |size, byte| size << 7 | usize::from(byte & 0x7f));
                self.skip = 10 + size + if self.head[5] & 0x10 != 0 { 10 } else { 0 };
            }
            std::mem::take(&mut self.head)
        };
        let skipped = self.skip.min(bytes.len());
        self.skip -= skipped;
        bytes.drain(..skipped);
        bytes
    }
}

/// A random UUID, which Volcano Engine wants for each request.
pub(crate) fn request_id() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| error.to_string())?;
    bytes[6] = bytes[6] & 0x0f | 0x40;
    bytes[8] = bytes[8] & 0x3f | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..]))
}

/// `text` in pieces of at most `max` UTF-16 code units: at the end of a sentence where one fits, else at a comma or
/// a space, else at a character. Pieces with nothing to say are left out.
pub fn split(text: &str, max: usize) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut rest = text.trim();
    while !rest.is_empty() {
        let mut units = 0;
        let mut fits = rest.len();
        let (mut sentence, mut pause) = (None, None);
        for (at, character) in rest.char_indices() {
            units += character.len_utf16();
            if units > max {
                fits = at;
                break;
            }
            let after = at + character.len_utf8();
            if matches!(character, '.' | '!' | '?' | '。' | '！' | '？' | '…' | '\n') {
                sentence = Some(after);
            } else if matches!(character, ',' | ';' | ':' | '，' | '；' | '：' | '、' | ' ') {
                pause = Some(after);
            }
        }
        let cut = match fits {
            _ if fits == rest.len() => fits,
            // Not even one character fits: it goes alone rather than never.
            0 => rest.chars().next().map_or(0, char::len_utf8),
            _ => sentence.or(pause).unwrap_or(fits),
        };
        let (piece, after) = rest.split_at(cut);
        if piece.chars().any(char::is_alphanumeric) {
            pieces.push(piece.trim());
        }
        rest = after.trim_start();
    }
    pieces
}

/// Each Character's voice file (`characters/<id>/voice.env`), so a spoken reply is in the very voice its lines
/// were made with: `ENGINE=elevenlabs` voices are the ones in our ElevenLabs account, which the voice service uses.
pub(crate) const VOICE_FILES: &[(&str, &str)] = &[
    ("ada", include_str!("../../characters/ada/voice.env")),
    ("dave", include_str!("../../characters/dave/voice.env")),
    ("hank", include_str!("../../characters/hank/voice.env")),
    ("kai", include_str!("../../characters/kai/voice.env")),
    ("luna", include_str!("../../characters/luna/voice.env")),
    ("mei", include_str!("../../characters/mei/voice.env")),
    ("wanwanxiaohe", include_str!("../../characters/wanwanxiaohe/voice.env")),
];
/// The built-in default Character speaks with ElevenLabs' Jessica (voices/README.md); so does no Character pack, or
/// one these files don't know.
const DEFAULT_VOICE: &str = "elevenlabs:cgSgspJ2msm6clMCkdW9";
/// ElevenLabs' stock Sarah and George, voices of no Character: one whose voice the service can't speak (a Doubao
/// 2.0 voice, which needs another resource id) gets the one of its `GENDER`, rather than another Character's voice.
const STOCK_FEMALE: &str = "elevenlabs:EXAVITQu4vr4xnSDxMaL";
const STOCK_MALE: &str = "elevenlabs:JBFqnCBsd6RMkjVDRZzb";

/// The voice service's name for `character`'s voice (the id the box reports in `VOICES`), as `<vendor>:<voice>`.
/// The model is multilingual, so a reply in another language than the Character's keeps the Character's voice.
/// A Character made in the App ([`crate::making`]) speaks in the voice it was made with.
pub fn voice_for(character: Option<&str>) -> String {
    voice_among(character, crate::making::DIR.get().map(PathBuf::as_path))
}

/// [`voice_for`], with the Characters made in `made`.
fn voice_among(character: Option<&str>, made: Option<&Path>) -> String {
    let Some(file) = character.and_then(|character| VOICE_FILES.iter().find(|(id, _)| *id == character)) else {
        return character.zip(made).and_then(|(id, made)| crate::making::voice_of(made, id)).unwrap_or_else(|| DEFAULT_VOICE.to_owned());
    };
    let field = |key: &str| voice_field(file.1, key);
    match (field("ENGINE"), field("VOICE")) {
        (Some("elevenlabs"), Some(voice)) => format!("elevenlabs:{voice}"),
        // The service speaks Doubao's 1.0 voices only.
        (Some("volc"), Some(voice)) if field("VOLC_RESOURCE_ID").is_none() => format!("volc:{voice}"),
        _ if field("GENDER") == Some("male") => STOCK_MALE.to_owned(),
        _ => STOCK_FEMALE.to_owned(),
    }
}

/// One `KEY=value` of a `voice.env`.
pub(crate) fn voice_field<'a>(file: &'a str, key: &str) -> Option<&'a str> {
    file.lines().find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
}

/// The synthesizer: the user's own keys, and the voice service the environment configures, if any.
pub fn from_env(session: Option<Arc<Account>>, keys: Arc<VoiceKeys>) -> Arc<dyn Synthesizer> {
    let service = configure(service_base(std::env::var(BASE_URL_VARIABLE).ok(), crate::relay::release()), session);
    Arc::new(Synthesis { service, keys, elevenlabs_url: ELEVENLABS_URL.to_owned(), volc_url: VOLC_URL.to_owned() })
}

/// The voice service's base URL, which speech recognition ([`crate::speech`]) uses too: the variable's, else
/// staging outside a release. None when it is `off`, not http(s), or a release has none.
pub fn service_base(base_url: Option<String>, release: bool) -> Option<String> {
    let base_url = match base_url.map(|url| url.trim().to_owned()).filter(|url| !url.is_empty()) {
        Some(url) if url == "off" => return None,
        Some(url) => url,
        None if release => return None,
        None => STAGING_URL.to_owned(),
    };
    if !base_url.starts_with("https://") && !base_url.starts_with("http://") {
        warn!(%base_url, "the voice service's base URL is not http(s); it is not used");
        return None;
    }
    Some(base_url)
}

fn configure(base_url: Option<String>, session: Option<Arc<Account>>) -> Option<VoiceService> {
    let base_url = base_url?;
    info!(%base_url, "speech synthesis: the voice service");
    Some(VoiceService { base_url, session })
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::net::TcpListener;

    use super::*;

    #[test]
    fn each_character_speaks_in_the_voice_of_its_lines() {
        assert_eq!(voice_for(Some("ada")), "elevenlabs:tcSsCJFYrsHpK5rCQZMf");
        assert_eq!(voice_for(Some("hank")), "elevenlabs:U6jH23hcPh0zH17nf3DT");
        assert_eq!(voice_for(Some("wanwanxiaohe")), "volc:zh_female_wanwanxiaohe_moon_bigtts", "a Doubao 1.0 voice");
        assert_eq!(voice_for(Some("robot")), DEFAULT_VOICE);
        assert_eq!(voice_for(None), DEFAULT_VOICE, "no Character pack");
        assert_eq!(voice_for(Some("hsiaoyu")), DEFAULT_VOICE, "an old voice pack");
    }

    #[test]
    fn a_made_character_speaks_in_the_voice_it_was_made_with() {
        let made = crate::voice_keys::tests::scratch("made-voice");
        std::fs::create_dir_all(made.join("owl-1a2b3c4d")).unwrap();
        let manifest = r#"{"name":"Owl","base":"ada","voice":"wanwanxiaohe","lines":[{"occasion":"done","text":"好了。"}]}"#;
        std::fs::write(made.join("owl-1a2b3c4d/manifest.json"), manifest).unwrap();
        assert_eq!(voice_among(Some("owl-1a2b3c4d"), Some(&made)), "volc:zh_female_wanwanxiaohe_moon_bigtts");
        assert_eq!(voice_among(Some("ada"), Some(&made)), "elevenlabs:tcSsCJFYrsHpK5rCQZMf", "a preset stays a preset");
        assert_eq!(voice_among(Some("owl-0000"), Some(&made)), DEFAULT_VOICE, "not made here");
        assert_eq!(voice_among(Some("../owl-1a2b3c4d"), Some(&made)), DEFAULT_VOICE);
        assert_eq!(voice_among(Some("owl-1a2b3c4d"), None), DEFAULT_VOICE);
        let _ = std::fs::remove_dir_all(made);
    }

    #[test]
    fn long_text_is_split_at_sentences_then_pauses_then_characters() {
        assert_eq!(split("  Short one.  ", 1000), ["Short one."]);
        assert_eq!(split("One. Two! Three?", 10), ["One. Two!", "Three?"]);
        assert_eq!(split("第一句。第二句。第三句。", 8), ["第一句。第二句。", "第三句。"]);
        assert_eq!(split("one two three four", 9), ["one two", "three", "four"]);
        assert_eq!(split("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        // A character outside the BMP is two units in JavaScript, which is how the service counts.
        assert_eq!(split("𠀀𠀀𠀀", 4), ["𠀀𠀀", "𠀀"]);
        assert!(split("😀", 4).is_empty(), "nothing to say");
        assert!(split(" ... ", 10).is_empty(), "nothing to say");
        let long = "This sentence is about forty characters. ".repeat(60);
        let pieces = split(&long, MAX_TEXT);
        assert!(pieces.len() > 1);
        assert!(pieces.iter().all(|piece| piece.encode_utf16().count() <= MAX_TEXT && piece.ends_with('.')));
    }

    #[test]
    fn the_service_is_staging_outside_a_release_and_can_be_set_or_switched_off() {
        let base = |url: Option<&str>, release| configure(service_base(url.map(str::to_owned), release), None).is_some();
        assert!(base(None, false));
        assert!(!base(None, true), "no production voice service yet");
        assert!(base(Some("https://api.korekore.ai"), true));
        assert!(!base(Some("off"), false));
        assert!(!base(Some("api.korekore.ai"), false));
    }

    /// Serves one canned HTTP answer on a local port; returns the base URL and the request it got.
    fn serve(answer: Vec<u8>) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 4096];
            while !String::from_utf8_lossy(&request).contains("}") {
                let read = socket.read(&mut buffer).unwrap();
                request.extend_from_slice(&buffer[..read]);
            }
            socket.write_all(&answer).unwrap();
            String::from_utf8_lossy(&request).into_owned()
        });
        (base, handle)
    }

    fn chunked(pieces: &[&[u8]], finished: bool) -> Vec<u8> {
        let mut out = b"HTTP/1.1 200 OK\r\nContent-Type: audio/mpeg\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
        for piece in pieces {
            out.extend_from_slice(format!("{:x}\r\n", piece.len()).as_bytes());
            out.extend_from_slice(piece);
            out.extend_from_slice(b"\r\n");
        }
        if finished {
            out.extend_from_slice(b"0\r\n\r\n");
        }
        out
    }

    fn collect(base: String) -> (Result<(), String>, Vec<u8>) {
        let service = VoiceService { base_url: base, session: None };
        let mut audio = Vec::new();
        let result = service.synthesize("Hello there.", "en-US-JennyNeural", &mut |piece| {
            audio.extend_from_slice(piece);
            true
        });
        (result, audio)
    }

    #[test]
    fn the_request_is_json_and_the_audio_comes_through() {
        let (base, server) = serve(chunked(&[b"ID3abc", b"\xff\xf3def"], true));
        let (result, audio) = collect(format!("{base}/"));
        assert_eq!(result, Ok(()));
        assert_eq!(audio, b"ID3abc\xff\xf3def");
        let request = server.join().unwrap();
        assert!(request.starts_with("POST /v1/voice/speak HTTP/1.1\r\n"), "{request}");
        assert!(request.to_ascii_lowercase().contains("content-type: application/json"));
        assert!(request.ends_with(r#"{"text":"Hello there.","voice":"en-US-JennyNeural"}"#), "{request}");
    }

    #[test]
    fn the_session_goes_along_and_a_401_forgets_it() {
        let dir = std::env::temp_dir().join(format!("vibebuddy-synthesis-{}", std::process::id()));
        let session = Arc::new(crate::account::signed_in_for_tests(&dir, "tok_xyz"));
        let (base, server) = serve(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 24\r\nConnection: close\r\n\r\n{\"error\":\"unauthorized\"}".to_vec());
        let service = VoiceService { base_url: base, session: Some(session.clone()) };
        let result = service.synthesize("Hi.", "elevenlabs:x", &mut |_| true);
        assert!(result.unwrap_err().contains("signed_out"));
        assert!(server.join().unwrap().to_ascii_lowercase().contains("authorization: bearer tok_xyz"));
        assert!(session.token().is_none());
        // Signed out, nothing more is sent.
        assert_eq!(service.synthesize("Hi.", "elevenlabs:x", &mut |_| true), Err("not signed in".to_owned()));
    }

    fn own_keys(keys: &[(Provider, &str)]) -> Arc<VoiceKeys> {
        let keys_file = Arc::new(VoiceKeys::new(Some(&crate::voice_keys::tests::scratch("synthesis"))));
        let changes: Vec<_> = keys.iter().map(|(provider, key)| (*provider, Some((*key).to_owned()))).collect();
        keys_file.set(&changes).unwrap();
        keys_file
    }

    fn synthesis(service: Option<String>, keys: Arc<VoiceKeys>, elevenlabs: &str, volc: &str) -> Synthesis {
        Synthesis {
            service: service.map(|base_url| VoiceService { base_url, session: None }),
            keys,
            elevenlabs_url: elevenlabs.to_owned(),
            volc_url: volc.to_owned(),
        }
    }

    fn speak_with(synthesis: &Synthesis, voice: &str) -> (Result<(), String>, Vec<u8>) {
        let mut audio = Vec::new();
        let result = synthesis.synthesize("你好。", voice, &mut |piece| {
            audio.extend_from_slice(piece);
            true
        });
        (result, audio)
    }

    #[test]
    fn an_own_key_goes_to_its_provider_only_and_other_voices_to_the_service() {
        let keys = own_keys(&[(Provider::ElevenLabs, "sk_own_eleven")]);
        let (eleven, eleven_server) = serve(chunked(&[b"\xff\xf3own"], true));
        let (service, service_server) = serve(chunked(&[b"\xff\xf3ours"], true));
        // Nothing listens on port 9: a reply sent there would fail.
        let synthesis = synthesis(Some(service), keys, &eleven, "http://127.0.0.1:9");

        assert_eq!(speak_with(&synthesis, "elevenlabs:voice1"), (Ok(()), b"\xff\xf3own".to_vec()));
        let request = eleven_server.join().unwrap();
        assert!(request.starts_with("POST /v1/text-to-speech/voice1/stream?output_format=mp3_24000_48 HTTP/1.1\r\n"), "{request}");
        assert!(request.to_ascii_lowercase().contains("xi-api-key: sk_own_eleven"));
        assert!(request.contains(r#""model_id":"eleven_flash_v2_5""#) && request.contains(r#""text":"你好。""#), "{request}");

        // No Volcano Engine key: that voice goes to the service, with no key of the user's.
        assert_eq!(speak_with(&synthesis, "volc:zh_female_wanwanxiaohe_moon_bigtts"), (Ok(()), b"\xff\xf3ours".to_vec()));
        let request = service_server.join().unwrap();
        assert!(request.starts_with("POST /v1/voice/speak HTTP/1.1\r\n"), "{request}");
        assert!(!request.contains("sk_own_eleven"));
    }

    #[test]
    fn volcano_engine_mp3_comes_without_its_id3_tag() {
        let keys = own_keys(&[(Provider::Volc, "volc_own_key")]);
        // A 5-byte tag (syncsafe size 0x0005), cut across two objects.
        let mp3 = b"ID3\x04\x00\x00\x00\x00\x00\x05TAGGG\xff\xf3frame";
        let encode = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);
        let objects = format!(
            r#"{{"code":0,"data":"{}"}}{{"code":0,"data":"{}"}}{{"code":0,"data":null}}{{"code":20000000,"message":"ok"}}"#,
            encode(&mp3[..7]),
            encode(&mp3[7..])
        );
        let (volc, server) = serve(chunked(&[&objects.as_bytes()[..30], &objects.as_bytes()[30..]], true));
        let synthesis = synthesis(None, keys, "http://127.0.0.1:9", &volc);
        assert!(synthesis.speaks("volc:zh_female_wanwanxiaohe_moon_bigtts"));
        assert!(!synthesis.speaks("elevenlabs:voice1"), "no service and no ElevenLabs key");
        assert_eq!(speak_with(&synthesis, "volc:zh_female_wanwanxiaohe_moon_bigtts"), (Ok(()), b"\xff\xf3frame".to_vec()));
        let request = server.join().unwrap();
        let lower = request.to_ascii_lowercase();
        assert!(lower.contains("x-api-key: volc_own_key") && lower.contains("x-api-resource-id: seed-tts-1.0"), "{request}");
        assert!(request.contains(r#""speaker":"zh_female_wanwanxiaohe_moon_bigtts""#) && request.contains(r#""format":"mp3""#) && request.contains(r#""sample_rate":24000"#), "{request}");
    }

    #[test]
    fn a_refused_own_key_is_reported_and_the_reply_stays_silent() {
        let keys = own_keys(&[(Provider::ElevenLabs, "sk_bad")]);
        let (eleven, server) = serve(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".to_vec());
        // The service is there, but a refused own key must not fall back to it: port 9 would answer nothing.
        let synthesis = synthesis(Some("http://127.0.0.1:9".to_owned()), keys.clone(), &eleven, "http://127.0.0.1:9");
        let (result, audio) = speak_with(&synthesis, "elevenlabs:voice1");
        server.join().unwrap();
        let error = result.unwrap_err();
        assert!(error.contains("ElevenLabs refused your own key (401)") && !error.contains("sk_bad"), "{error}");
        assert!(audio.is_empty());
        assert_eq!(keys.status().elevenlabs, crate::voice_keys::KeyStatus { set: true, refused: true });
        assert!(!keys.status().volc.refused);
    }

    #[test]
    fn an_id3_tag_is_dropped_however_it_is_cut() {
        let stream = b"ID3\x03\x00\x00\x00\x00\x01\x00".iter().copied().chain(std::iter::repeat_n(b'T', 128)).chain(*b"\xff\xf3abc").collect::<Vec<_>>();
        for cut in [1, 4, 10, 100, 138, 139] {
            let mut tag = Id3::default();
            let mut out = tag.strip(&stream[..cut]);
            out.extend(tag.strip(&stream[cut..]));
            assert_eq!(out, b"\xff\xf3abc", "cut at {cut}");
        }
        let mut none = Id3::default();
        assert_eq!(none.strip(b"\xff\xf3abcdefghij"), b"\xff\xf3abcdefghij");
    }

    #[test]
    fn a_stream_cut_short_or_an_error_status_is_a_failure() {
        // The connection drops before the chunked body's end: what came is kept, but it failed.
        let (base, server) = serve(chunked(&[b"ID3abc"], false));
        let (result, audio) = collect(base);
        server.join().unwrap();
        assert!(result.is_err(), "{result:?}");
        assert_eq!(audio, b"ID3abc");

        let (base, server) = serve(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 15\r\n\r\nupstream failed".to_vec());
        let (result, audio) = collect(base);
        server.join().unwrap();
        assert_eq!(result.unwrap_err(), "the voice service answered 502: upstream failed");
        assert!(audio.is_empty());
    }
}
