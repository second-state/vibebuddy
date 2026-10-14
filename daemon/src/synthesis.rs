//! Speech synthesis for spoken replies (ADR-0017): a reply's text in, an MP3 stream out, piece by piece as the
//! service makes it.
//!
//! Behind [`Synthesizer`] so the backend can change. There is one: the voice service in `vibebuddy-cloud`,
//! `POST {base}/v0/voice/speak` with `{"text","voice"}`, answering a chunked `audio/mpeg` stream (MPEG-2 Layer III,
//! 24 kHz mono, 48 kbps) that starts as soon as there is audio. Builds other than releases use staging; a release
//! has none until the production service is deployed. `VIBEBUDDY_TTS_BASE_URL` sets the base by hand, or `off`.

use std::io::Read;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tracing::{info, warn};

/// The voice service's base URL, or `off`. Unset: staging, except in a release.
pub const BASE_URL_VARIABLE: &str = "VIBEBUDDY_TTS_BASE_URL";
pub const STAGING_URL: &str = "https://api-staging.korekore.ai";
/// The most text the service takes in one request, in UTF-16 code units (JavaScript's `length`).
pub const MAX_TEXT: usize = 1000;
/// A request gets this long for its whole answer; 1000 characters is about a minute of speech, made faster than
/// it plays.
const TIMEOUT: Duration = Duration::from_secs(120);
/// No reply needs more: ten minutes at 48 kbps.
const MAX_AUDIO: usize = 4 * 1024 * 1024;

/// Turns one reply into speech. Blocking: it is called on a blocking thread.
pub trait Synthesizer: Send + Sync {
    /// Calls `on_audio` with each piece of the MP3 as it arrives (not on frame boundaries); `on_audio` returns false
    /// to stop. An error after some audio means the stream was cut short.
    fn synthesize(&self, text: &str, voice: &str, on_audio: &mut dyn FnMut(&[u8]) -> bool) -> Result<(), String>;
}

/// The voice service.
pub struct VoiceService {
    base_url: String,
}

impl VoiceService {
    fn url(&self) -> String {
        format!("{}/v0/voice/speak", self.base_url.trim_end_matches('/'))
    }

    fn speak(&self, text: &str, voice: &str, on_audio: &mut dyn FnMut(&[u8]) -> bool) -> Result<bool, String> {
        let url = self.url();
        let host = url.split('/').nth(2).unwrap_or_default().to_owned();
        let body = json!({"text": text, "voice": voice}).to_string();
        // The seam for the account session: its Authorization header goes here once the service asks for one.
        // One agent for every reply, so its connection to the service is reused: a fresh TLS handshake cost about
        // half a second a reply.
        static AGENT: std::sync::OnceLock<ureq::Agent> = std::sync::OnceLock::new();
        let mut response = AGENT
            .get_or_init(|| crate::speech::agent(TIMEOUT))
            .post(&url)
            .header("Content-Type", "application/json")
            .send(body.as_bytes())
            .map_err(|error| format!("calling {host}: {error}"))?;
        let status = response.status().as_u16();
        if status != 200 {
            let answer = response.body_mut().with_config().limit(1024).read_to_string().unwrap_or_default();
            return Err(format!("the voice service answered {status}: {}", answer.trim()));
        }
        let mut reader = response.into_body().into_reader();
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
const VOICE_FILES: &[(&str, &str)] = &[
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
/// Until the voice service speaks Doubao, a Character whose voice is there (the Chinese ones) borrows Mei's: she
/// is Chinese-American, and ElevenLabs' multilingual model reads Chinese in her voice.
const NOT_YET_SPOKEN: &str = "mei";

/// The voice service's name for `character`'s voice (the id the box reports in `VOICES`), as `<vendor>:<voice>`.
/// The model is multilingual, so a reply in another language than the Character's keeps the Character's voice.
pub fn voice_for(character: Option<&str>) -> String {
    let voice_of = |character: &str| {
        let file = VOICE_FILES.iter().find(|(id, _)| *id == character)?.1;
        let field = |key: &str| file.lines().find_map(|line| line.strip_prefix(key)?.strip_prefix('='));
        Some((field("ENGINE")?, field("VOICE")?))
    };
    match character.and_then(voice_of) {
        Some(("elevenlabs", voice)) => format!("elevenlabs:{voice}"),
        Some(_) => voice_of(NOT_YET_SPOKEN).map_or(DEFAULT_VOICE.to_owned(), |(_, voice)| format!("elevenlabs:{voice}")),
        None => DEFAULT_VOICE.to_owned(),
    }
}

/// The synthesizer the environment configures, if any.
pub fn from_env() -> Option<Arc<dyn Synthesizer>> {
    configure(std::env::var(BASE_URL_VARIABLE).ok(), crate::relay::release())
}

fn configure(base_url: Option<String>, release: bool) -> Option<Arc<dyn Synthesizer>> {
    let base_url = match base_url.map(|url| url.trim().to_owned()).filter(|url| !url.is_empty()) {
        Some(url) if url == "off" => return None,
        Some(url) => url,
        None if release => return None,
        None => STAGING_URL.to_owned(),
    };
    if !base_url.starts_with("https://") && !base_url.starts_with("http://") {
        warn!(%base_url, "the voice service's base URL is not http(s); spoken replies stay text only");
        return None;
    }
    info!(%base_url, "speech synthesis: the voice service");
    Some(Arc::new(VoiceService { base_url }))
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
        assert_eq!(voice_for(Some("wanwanxiaohe")), voice_for(Some("mei")), "Doubao isn't spoken yet");
        assert_eq!(voice_for(Some("robot")), DEFAULT_VOICE);
        assert_eq!(voice_for(None), DEFAULT_VOICE, "no Character pack");
        assert_eq!(voice_for(Some("hsiaoyu")), DEFAULT_VOICE, "an old voice pack");
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
        let base = |url: Option<&str>, release| {
            configure(url.map(str::to_owned), release).map(|_| ()).is_some()
        };
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
        let service = VoiceService { base_url: base };
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
        assert!(request.starts_with("POST /v0/voice/speak HTTP/1.1\r\n"), "{request}");
        assert!(request.to_ascii_lowercase().contains("content-type: application/json"));
        assert!(request.ends_with(r#"{"text":"Hello there.","voice":"en-US-JennyNeural"}"#), "{request}");
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
