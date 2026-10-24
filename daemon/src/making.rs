//! Making a Character (ADR-0020, docs/characters.md): the lines the user wrote, spoken in a voice from
//! `characters/voices.tsv`, made into audio pools the App packs with a look.
//!
//! `POST /v1/characters` starts one job at a time on its own thread. Each line is synthesized as 16 kHz PCM: with the
//! user's own key for the voice's provider, from that provider; otherwise from the voice service with `"for":"character"`
//! and the account's session, which only the Paid plan gets. Then, as `tools/make-character.sh` does with ffmpeg, it is
//! trimmed of leading and trailing silence and normalized to a -1 dBFS peak. Everything lands in
//! `config_dir()/characters/<id>/`, laid out as `make-character.sh` lays out `characters/<id>/`:
//!
//! - `audio/<occasion>/<prefix>-<hash>.pcm` (and `.txt`), and `audio-address/<form>/...` for the lines with
//!   `{address}`, said once with each form of the voice's language from `characters/addresses.tsv`;
//! - `pack.bin` and `address/<form>.bin`, Character packs exactly as `tools/character_pack.py` builds them (16 kHz IMA
//!   ADPCM, no look), written once every line is made: the App reads them as it reads a preset's;
//! - `manifest.json`: the name, base, voice and lines asked for.
//!
//! A line already made is never made again, so a refusal (`plan`, `monthly`, `global_cap`, `signed_out`, or the
//! provider refusing the own key, `own_key`) stops the job, keeps the lines made, and starting the same Character again
//! (its `id`) picks up where it stopped. Progress, the state and why it stopped are `making` in `/v1/status`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha1::{Digest, Sha1};
use tokio::sync::watch;
use tracing::{info, warn};
use vibebuddy_firmware_core::adpcm::Encoder;
use vibebuddy_firmware_core::audio::Occasion;

use crate::account::Account;
use crate::synthesis::{ELEVENLABS_MODEL, MAX_TEXT, VOLC_RESOURCE_ID};
use crate::voice_keys::{Provider, VoiceKeys};

const VOICES: &str = include_str!("../../characters/voices.tsv");
const ADDRESSES: &str = include_str!("../../characters/addresses.tsv");
pub const MANIFEST: &str = "manifest.json";
/// The most lines a pack with a look has room for, which is what the App makes of these.
const MAX_LINES: usize = (1008 - 128) / 8;
/// No line needs more: about a minute of 16 kHz PCM.
const MAX_AUDIO: u64 = 2 * 1024 * 1024;
const SAMPLE_RATE: u32 = 16_000;

/// A voice a made Character can speak with, a row of `characters/voices.tsv`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Voice {
    pub id: String,
    pub provider: String,
    /// The provider's id for it.
    pub voice: String,
    pub language: String,
    pub gender: String,
    pub name: String,
}

pub fn voices() -> Vec<Voice> {
    rows(VOICES)
        .filter_map(|row| match row[..] {
            [id, provider, voice, language, gender, name] => Some(Voice {
                id: id.to_owned(),
                provider: provider.to_owned(),
                voice: voice.to_owned(),
                language: language.to_owned(),
                gender: gender.to_owned(),
                name: name.to_owned(),
            }),
            _ => None,
        })
        .collect()
}

/// Each preset's lines (`characters/<id>/lines.tsv`), which a made Character starts from in the App.
const PRESET_LINES: &[(&str, &str)] = &[
    ("ada", include_str!("../../characters/ada/lines.tsv")),
    ("dave", include_str!("../../characters/dave/lines.tsv")),
    ("hank", include_str!("../../characters/hank/lines.tsv")),
    ("kai", include_str!("../../characters/kai/lines.tsv")),
    ("luna", include_str!("../../characters/luna/lines.tsv")),
    ("mei", include_str!("../../characters/mei/lines.tsv")),
    ("wanwanxiaohe", include_str!("../../characters/wanwanxiaohe/lines.tsv")),
];

/// The lines of preset `id`, in `lines.tsv` order; None for anything not a preset.
pub fn preset_lines(id: &str) -> Option<Vec<Line>> {
    let (_, tsv) = PRESET_LINES.iter().find(|(preset, _)| *preset == id)?;
    Some(
        rows(tsv)
            .filter_map(|row| match row[..] {
                [occasion, text] => Some(Line { occasion: occasion.to_owned(), text: text.to_owned() }),
                _ => None,
            })
            .collect(),
    )
}

/// A TSV's rows, without blank ones and `#` comments.
fn rows(tsv: &str) -> impl Iterator<Item = Vec<&str>> {
    tsv.lines().filter(|line| !line.trim().is_empty() && !line.starts_with('#')).map(|line| line.split('\t').collect())
}

/// The forms of address of `language`: id and the words spoken.
fn forms(language: &str) -> Vec<(&'static str, &'static str)> {
    rows(ADDRESSES)
        .filter_map(|row| match row[..] {
            [lang, form, words] if lang == language => Some((form, words)),
            _ => None,
        })
        .collect()
}

/// A line said with no form of address: `{address}` and the comma or space that joins it go, as in
/// `tools/make-character.sh`.
pub fn without_address(text: &str) -> String {
    static PATTERNS: std::sync::OnceLock<[(Regex, &str); 4]> = std::sync::OnceLock::new();
    // make-character.sh's lookaheads, as a captured character put back.
    let patterns = PATTERNS.get_or_init(|| {
        [
            (Regex::new(r"[，,]\s*\{address\}([。！？.!?，,]|$)").unwrap(), "$1"),
            (Regex::new(r"\s\{address\}([,.!?])").unwrap(), "$1"),
            (Regex::new(r"\{address\}[，,]\s*").unwrap(), ""),
            (Regex::new(r"\s?\{address\}").unwrap(), ""),
        ]
    });
    patterns.iter().fold(text.to_owned(), |text, (pattern, with)| pattern.replace_all(&text, *with).into_owned())
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Line {
    pub occasion: String,
    pub text: String,
}

/// `POST /v1/characters`, and the manifest without the `id`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Request {
    /// A Character made before, to make again; None for a new one.
    #[serde(default, skip_serializing)]
    pub id: Option<String>,
    pub name: String,
    /// The preset it started from.
    pub base: String,
    /// An id from `characters/voices.tsv`.
    pub voice: String,
    pub lines: Vec<Line>,
}

/// One piece of audio to make: where it goes under the Character's directory, and what it says.
#[derive(Debug, PartialEq)]
struct Piece {
    path: PathBuf,
    text: String,
}

/// What `make-character.sh` synthesizes for these lines, named as it names them: a line with `{address}` once without a
/// form into `audio/` and once with each form into `audio-address/<form>/`, prefixed `0-<row>` so they come first in
/// their pool in row order; any other line once into `audio/`, prefixed `1`.
fn pieces(lines: &[Line], forms: &[(&str, &str)]) -> Vec<Piece> {
    let piece = |into: PathBuf, occasion: &str, prefix: &str, text: String| {
        let hash: String = Sha1::digest(text.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect();
        Piece { path: into.join(occasion).join(format!("{prefix}-{}.pcm", &hash[..10])), text }
    };
    let mut pieces = Vec::new();
    for (row, line) in lines.iter().enumerate() {
        if line.text.contains("{address}") {
            let prefix = format!("0-{:03}", row + 1);
            pieces.push(piece("audio".into(), &line.occasion, &prefix, without_address(&line.text)));
            for (form, words) in forms {
                let into = Path::new("audio-address").join(form);
                pieces.push(piece(into, &line.occasion, &prefix, line.text.replace("{address}", words)));
            }
        } else {
            pieces.push(piece("audio".into(), &line.occasion, "1", line.text.clone()));
        }
    }
    // The same text twice in a pool is one line, as one file.
    let mut seen = std::collections::HashSet::new();
    pieces.retain(|piece| seen.insert(piece.path.clone()));
    pieces
}

/// `making` in the status: the job running, or the last one.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MakingStatus {
    pub id: String,
    pub name: String,
    /// `making`, `done` or `failed`.
    pub state: &'static str,
    /// Pieces of audio made, of all there are: each line, and each form of address of those with one.
    pub done: usize,
    pub total: usize,
    /// Why it stopped, for the App to explain: `plan`, `monthly`, `global_cap`, `signed_out`, `own_key` or `internal`; None for
    /// anything else, which `error` describes.
    pub reason: Option<&'static str>,
    pub error: Option<String>,
}

/// A Character made here, in `GET /v1/characters`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MadeCharacter {
    pub id: String,
    pub name: String,
    pub base: String,
    pub voice: String,
    pub lines: usize,
    /// Its packs are made, for the lines it has now.
    pub complete: bool,
}

/// Where the Characters made here are, for [`voice_of`]; set once at start, never in tests.
pub static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// The voice service's name (`<provider>:<voice>`) for the voice of `id`, a Character made in `dir`.
pub fn voice_of(dir: &Path, id: &str) -> Option<String> {
    if !valid_id(id) {
        return None;
    }
    let manifest: Request = serde_json::from_str(&std::fs::read_to_string(dir.join(id).join(MANIFEST)).ok()?).ok()?;
    let voice = voices().into_iter().find(|voice| voice.id == manifest.voice)?;
    Some(format!("{}:{}", voice.provider, voice.voice))
}

/// Why a job can't start.
#[derive(Debug, PartialEq)]
pub enum StartError {
    Busy,
    Invalid(String),
    /// No config directory, or it can't be written.
    Unavailable(String),
}

/// What stopped a line.
#[derive(Debug, PartialEq)]
struct Stop {
    reason: Option<&'static str>,
    message: String,
}

impl Stop {
    fn because(reason: &'static str, message: impl Into<String>) -> Self {
        Self { reason: Some(reason), message: message.into() }
    }
}

impl From<String> for Stop {
    fn from(message: String) -> Self {
        Self { reason: None, message }
    }
}

pub struct Maker {
    /// `config_dir()/characters`; None without a config directory, and in tests unless one sets it.
    dir: Option<PathBuf>,
    /// The voice service's base URL, if there is one.
    service: Option<String>,
    account: Arc<Account>,
    keys: Arc<VoiceKeys>,
    /// The providers' base URLs; others only in tests.
    elevenlabs_url: String,
    volc_url: String,
    status: Mutex<Option<MakingStatus>>,
    changed: watch::Sender<()>,
}

impl Maker {
    pub fn new(dir: Option<PathBuf>, service: Option<String>, account: Arc<Account>, keys: Arc<VoiceKeys>) -> Self {
        Self {
            dir,
            service,
            account,
            keys,
            elevenlabs_url: crate::synthesis::ELEVENLABS_URL.to_owned(),
            volc_url: crate::synthesis::VOLC_URL.to_owned(),
            status: Mutex::new(None),
            changed: watch::channel(()).0,
        }
    }

    /// Changes with each step of a job.
    pub fn watch(&self) -> watch::Receiver<()> {
        self.changed.subscribe()
    }

    pub fn status(&self) -> Option<MakingStatus> {
        self.lock().clone()
    }

    /// The status, even after a job panicked holding it.
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<MakingStatus>> {
        self.status.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn report(&self, change: impl FnOnce(&mut MakingStatus)) {
        if let Some(status) = self.lock().as_mut() {
            change(status);
        }
        self.changed.send_replace(());
    }

    /// Runs a job; one that panics ends as failed, `internal`, rather than making forever.
    fn guard(&self, job: impl FnOnce()) {
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)).is_err() {
            self.report(|status| {
                status.state = "failed";
                status.reason = Some("internal");
                status.error = Some("making the Character stopped unexpectedly".to_owned());
            });
        }
    }

    /// The Characters made here, by id: what was asked, and whether its packs are made.
    pub fn list(&self) -> Vec<MadeCharacter> {
        let Some(entries) = self.dir.as_ref().and_then(|dir| std::fs::read_dir(dir).ok()) else { return Vec::new() };
        let mut made: Vec<MadeCharacter> = entries
            .flatten()
            .filter_map(|entry| {
                let id = entry.file_name().into_string().ok().filter(|id| valid_id(id))?;
                let manifest: Request = serde_json::from_str(&std::fs::read_to_string(entry.path().join(MANIFEST)).ok()?).ok()?;
                let complete = entry.path().join("pack.bin").is_file();
                Some(MadeCharacter { id, name: manifest.name, base: manifest.base, voice: manifest.voice, lines: manifest.lines.len(), complete })
            })
            .collect();
        made.sort_by(|a, b| a.id.cmp(&b.id));
        made
    }

    /// Checks the request and starts making it on a thread of its own; answers the job's first status.
    pub fn start(self: &Arc<Self>, request: Request) -> Result<MakingStatus, StartError> {
        let (id, voice) = self.begin(&request)?;
        let maker = self.clone();
        std::thread::spawn(move || maker.guard(|| maker.make(&id, &voice, &request)));
        Ok(self.status().expect("begun"))
    }

    /// Checks the request, settles the Character's id and marks the job as making.
    fn begin(&self, request: &Request) -> Result<(String, Voice), StartError> {
        let dir = self.dir.as_ref().ok_or_else(|| StartError::Unavailable("no config directory".to_owned()))?;
        let voice = check(request).map_err(StartError::Invalid)?;
        let id = match &request.id {
            Some(id) if valid_id(id) && dir.join(id).join(MANIFEST).is_file() => id.clone(),
            Some(id) => return Err(StartError::Invalid(format!("{id} is not a Character made here"))),
            None => new_id(&request.name).map_err(StartError::Unavailable)?,
        };
        let mut status = self.lock();
        if status.as_ref().is_some_and(|status| status.state == "making") {
            return Err(StartError::Busy);
        }
        *status = Some(MakingStatus { id: id.clone(), name: request.name.trim().to_owned(), state: "making", done: 0, total: 0, reason: None, error: None });
        drop(status);
        self.changed.send_replace(());
        Ok((id, voice))
    }

    /// Makes every line not made yet, then the packs; reports each step.
    fn make(&self, id: &str, voice: &Voice, request: &Request) {
        let dir = self.dir.as_ref().expect("begun").join(id);
        match self.make_in(&dir, id, voice, request) {
            Ok(()) => {
                info!(id, "made a Character");
                self.report(|status| status.state = "done");
            }
            Err(stop) => {
                warn!(id, reason = stop.reason, error = %stop.message, "making a Character stopped");
                self.report(|status| {
                    status.state = "failed";
                    status.reason = stop.reason;
                    status.error = Some(stop.message);
                });
            }
        }
    }

    fn make_in(&self, dir: &Path, id: &str, voice: &Voice, request: &Request) -> Result<(), Stop> {
        let io = |error: std::io::Error| Stop::from(format!("writing {}: {error}", dir.display()));
        // Lines made in another voice are no use to this one.
        let before: Option<Request> = std::fs::read_to_string(dir.join(MANIFEST)).ok().and_then(|text| serde_json::from_str(&text).ok());
        if before.is_some_and(|before| before.voice != request.voice) {
            for gone in ["audio", "audio-address"] {
                let _ = std::fs::remove_dir_all(dir.join(gone));
            }
        }
        std::fs::create_dir_all(dir).map_err(io)?;
        write(&dir.join(MANIFEST), serde_json::to_string_pretty(request).expect("serializes").as_bytes()).map_err(io)?;
        // The packs are of the lines before; they come back once these are made.
        let _ = std::fs::remove_file(dir.join("pack.bin"));
        let _ = std::fs::remove_dir_all(dir.join("address"));

        let forms = forms(&voice.language);
        let pieces = pieces(&request.lines, &forms);
        let made = pieces.iter().filter(|piece| dir.join(&piece.path).is_file()).count();
        self.report(|status| (status.done, status.total) = (made, pieces.len()));
        for (done, piece) in (made + 1..).zip(pieces.iter().filter(|piece| !dir.join(&piece.path).is_file())) {
            let samples = trim_and_normalize(&self.synthesize(voice, &piece.text)?).ok_or_else(|| format!("the line came back silent: {}", piece.text))?;
            let path = dir.join(&piece.path);
            std::fs::create_dir_all(path.parent().expect("in a pool")).map_err(io)?;
            write(&path.with_extension("txt"), format!("{}\n", piece.text).as_bytes()).map_err(io)?;
            write(&path, &samples.iter().flat_map(|sample| sample.to_le_bytes()).collect::<Vec<_>>()).map_err(io)?;
            self.report(|status| status.done = done);
        }

        // Drop the audio of lines that were deleted or reworded, as make-character.sh does.
        let keep: std::collections::HashSet<PathBuf> = pieces.iter().map(|piece| dir.join(&piece.path)).collect();
        for root in ["audio", "audio-address"] {
            prune(&dir.join(root), &keep);
        }
        write(&dir.join("pack.bin"), &pack(id, &pools(&dir.join("audio")))?).map_err(io)?;
        for (form, _) in &forms {
            let audio = dir.join("audio-address").join(form);
            if audio.is_dir() {
                std::fs::create_dir_all(dir.join("address")).map_err(io)?;
                write(&dir.join("address").join(format!("{form}.bin")), &pack(id, &pools(&audio))?).map_err(io)?;
            }
        }
        Ok(())
    }

    /// One line as 16 kHz mono PCM: from the provider with the user's own key for it, else from the voice service.
    fn synthesize(&self, voice: &Voice, text: &str) -> Result<Vec<i16>, Stop> {
        let provider = Provider::named(&voice.provider).ok_or_else(|| format!("{} is not a provider", voice.provider))?;
        let bytes = match self.keys.key(provider) {
            Some(key) => self.own(provider, &voice.voice, &key, text)?,
            None => self.service(&format!("{}:{}", voice.provider, voice.voice), text)?,
        };
        Ok(pcm(&bytes))
    }

    fn own(&self, provider: Provider, voice: &str, key: &str, text: &str) -> Result<Vec<u8>, Stop> {
        let agent = agent();
        let (name, request) = match provider {
            Provider::ElevenLabs => {
                let url = format!("{}/v1/text-to-speech/{voice}/stream?output_format=pcm_16000", self.elevenlabs_url);
                let request = agent.post(&url).header("xi-api-key", key).header("Accept", "audio/pcm").header("Content-Type", "application/json");
                ("ElevenLabs", request.send(json!({"text": text, "model_id": ELEVENLABS_MODEL}).to_string().as_bytes()))
            }
            Provider::Volc => {
                let body = json!({
                    "user": {"uid": "vibebuddy"},
                    "req_params": {"text": text, "speaker": voice, "audio_params": {"format": "pcm", "sample_rate": SAMPLE_RATE}},
                });
                let request = agent
                    .post(&self.volc_url)
                    .header("X-Api-Key", key)
                    .header("X-Api-Resource-Id", VOLC_RESOURCE_ID)
                    .header("X-Api-Request-Id", &crate::synthesis::request_id()?)
                    .header("Content-Type", "application/json");
                ("Volcano Engine", request.send(body.to_string().as_bytes()))
            }
        };
        let mut response = request.map_err(|error| format!("calling {name}: {error}"))?;
        let status = response.status().as_u16();
        if matches!(status, 401 | 403) {
            self.keys.refused(provider, true);
            return Err(Stop::because("own_key", format!("{name} refused your own key ({status})")));
        }
        if status != 200 {
            let answer = response.body_mut().with_config().limit(1024).read_to_string().unwrap_or_default();
            return Err(format!("{name} answered {status}: {}", answer.trim().chars().take(200).collect::<String>()).into());
        }
        self.keys.refused(provider, false);
        match provider {
            Provider::ElevenLabs => read_all(&mut response, name),
            Provider::Volc => {
                let mut audio = Vec::new();
                crate::synthesis::read_volc(response.into_body().into_reader(), &mut |piece| {
                    audio.extend_from_slice(piece);
                    true
                })?;
                Ok(audio)
            }
        }
    }

    fn service(&self, voice: &str, text: &str) -> Result<Vec<u8>, Stop> {
        let base = self.service.as_deref().ok_or("no voice service, and no own key for this voice".to_owned())?;
        let token = self.account.token().ok_or_else(|| Stop::because("signed_out", "not signed in"))?;
        let url = format!("{}/v1/voice/speak", base.trim_end_matches('/'));
        let mut response = agent()
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Authorization", &format!("Bearer {token}"))
            .send(json!({"text": text, "voice": voice, "for": "character"}).to_string().as_bytes())
            .map_err(|error| format!("calling the voice service: {error}"))?;
        let status = response.status().as_u16();
        if status == 200 {
            return read_all(&mut response, "the voice service");
        }
        let answer = response.body_mut().with_config().limit(1024).read_to_string().unwrap_or_default();
        let reason = serde_json::from_str::<serde_json::Value>(&answer).ok().and_then(|value| value.get("reason")?.as_str().map(str::to_owned));
        Err(match (status, reason.as_deref()) {
            (401, _) => {
                self.account.forget();
                Stop::because("signed_out", "signed out")
            }
            (403, _) => Stop::because("plan", "making a Character with our voices needs the Paid plan"),
            (429, Some("global_cap")) => Stop::because("global_cap", "voice is paused for today"),
            (429, _) => Stop::because("monthly", "this month's Characters are used up"),
            _ => format!("the voice service answered {status}: {}", answer.trim()).into(),
        })
    }
}

fn agent() -> &'static ureq::Agent {
    static AGENT: std::sync::OnceLock<ureq::Agent> = std::sync::OnceLock::new();
    AGENT.get_or_init(|| crate::speech::agent(crate::synthesis::TIMEOUT))
}

fn read_all(response: &mut ureq::http::Response<ureq::Body>, from: &str) -> Result<Vec<u8>, Stop> {
    Ok(response.body_mut().with_config().limit(MAX_AUDIO).read_to_vec().map_err(|error| format!("reading audio from {from}: {error}"))?)
}

/// The voice, if the request can be made.
fn check(request: &Request) -> Result<Voice, String> {
    let name = request.name.trim();
    if name.is_empty() || name.chars().count() > 64 {
        return Err("a name must be 1 to 64 characters".to_owned());
    }
    if !crate::synthesis::VOICE_FILES.iter().any(|(id, _)| *id == request.base) {
        return Err(format!("{} is not a preset Character", request.base));
    }
    let voice = voices().into_iter().find(|voice| voice.id == request.voice).ok_or_else(|| format!("{} is not in voices.tsv", request.voice))?;
    if request.lines.is_empty() || request.lines.len() > MAX_LINES {
        return Err(format!("a Character has 1 to {MAX_LINES} lines"));
    }
    for line in &request.lines {
        if Occasion::from_name(&line.occasion).is_none() {
            return Err(format!("{} is not an occasion", line.occasion));
        }
        let units = line.text.encode_utf16().count();
        if line.text.trim().is_empty() || units > MAX_TEXT || line.text.contains(['\n', '\r', '\t']) {
            return Err(format!("a line must be one row of 1 to {MAX_TEXT} characters: {:?}", line.text));
        }
    }
    Ok(voice)
}

/// A made Character's id: lowercase letters, digits and dashes, at most 31, as a pack's id may be.
fn valid_id(id: &str) -> bool {
    (1..=31).contains(&id.len()) && id.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

/// The slug of the name and eight random hex digits.
fn new_id(name: &str) -> Result<String, String> {
    let mut slug = String::new();
    for character in name.to_lowercase().chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug: String = slug.trim_end_matches('-').chars().take(22).collect();
    let slug = if slug.is_empty() { "character" } else { slug.trim_end_matches('-') };
    let mut bytes = [0u8; 4];
    getrandom::fill(&mut bytes).map_err(|error| error.to_string())?;
    Ok(format!("{slug}-{:08x}", u32::from_le_bytes(bytes)))
}

/// Written whole or not at all, so a line cut short by a crash is made again.
fn write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let temporary = path.with_extension("part");
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(temporary, path)
}

fn prune(root: &Path, keep: &std::collections::HashSet<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            prune(&path, keep);
            let _ = std::fs::remove_dir(&path);
        } else if path.extension().is_some_and(|extension| extension == "pcm") && !keep.contains(&path) {
            let _ = std::fs::remove_file(path.with_extension("txt"));
            let _ = std::fs::remove_file(&path);
        }
    }
    let _ = std::fs::remove_dir(root);
}

/// Each occasion's lines in a directory as `tools/character_pack.py` reads them: `<occasion>/*.pcm` in name order.
fn pools(audio: &Path) -> Vec<Vec<Vec<i16>>> {
    Occasion::ALL
        .iter()
        .map(|occasion| {
            let mut files: Vec<PathBuf> = std::fs::read_dir(audio.join(occasion.name()))
                .map(|entries| entries.flatten().map(|entry| entry.path()).filter(|path| path.extension().is_some_and(|extension| extension == "pcm")).collect())
                .unwrap_or_default();
            files.sort();
            files
                .iter()
                .filter_map(|path| std::fs::read(path).ok())
                .map(|bytes| pcm(&bytes))
                .collect()
        })
        .collect()
}

/// 16-bit little-endian PCM as samples.
fn pcm(bytes: &[u8]) -> Vec<i16> {
    bytes.as_chunks::<2>().0.iter().map(|pair| i16::from_le_bytes(*pair)).collect()
}

/// TTS pads both ends with silence; trimmed at -50 dBFS, the box speaks as soon as it is asked to. Then the peak goes
/// to -1 dBFS. None when nothing is left.
fn trim_and_normalize(samples: &[i16]) -> Option<Vec<i16>> {
    let threshold = 32768.0 * 10f64.powf(-50.0 / 20.0);
    let loud = |sample: &i16| f64::from(sample.unsigned_abs()) > threshold;
    let start = samples.iter().position(loud)?;
    let end = samples.iter().rposition(loud)? + 1;
    let trimmed = &samples[start..end];
    let peak = f64::from(trimmed.iter().map(|sample| sample.unsigned_abs()).max()?);
    let gain = 32768.0 * 10f64.powf(-1.0 / 20.0) / peak;
    Some(trimmed.iter().map(|&sample| (f64::from(sample) * gain).round().clamp(-32768.0, 32767.0) as i16).collect())
}

/// IMA ADPCM with the box's own encoder, two samples a byte, low nibble first.
fn encode(samples: &[i16]) -> Vec<u8> {
    let mut encoder = Encoder::default();
    samples.chunks(2).map(|pair| if let [first, second] = pair { encoder.encode_pair(*first, *second) } else { encoder.encode(pair[0]) }).collect()
}

/// A Character pack with no look, byte for byte as `tools/character_pack.py` builds it.
fn pack(id: &str, pools: &[Vec<Vec<i16>>]) -> Result<Vec<u8>, String> {
    use vibebuddy_firmware_core::character_pack::{HEADER_BYTES, MAGIC, MAX_LINES};
    let mut header = vec![0u8; HEADER_BYTES];
    let mut payload = Vec::new();
    let mut line = 0usize;
    let put = |header: &mut Vec<u8>, at: usize, bytes: &[u8]| header[at..at + bytes.len()].copy_from_slice(bytes);
    for (slot, lines) in pools.iter().enumerate() {
        put(&mut header, 56 + slot * 4, &(if lines.is_empty() { 0 } else { line as u16 }).to_le_bytes());
        put(&mut header, 58 + slot * 4, &(lines.len() as u16).to_le_bytes());
        for samples in lines {
            if line == MAX_LINES {
                return Err(format!("more than {MAX_LINES} lines"));
            }
            put(&mut header, 128 + line * 8, &((HEADER_BYTES + payload.len()) as u32).to_le_bytes());
            put(&mut header, 132 + line * 8, &(samples.len() as u32).to_le_bytes());
            payload.extend(encode(samples));
            line += 1;
        }
    }
    put(&mut header, 0, MAGIC);
    put(&mut header, 4, &1u32.to_le_bytes());
    put(&mut header, 8, &(payload.len() as u32).to_le_bytes());
    put(&mut header, 12, &crc32fast::hash(&payload).to_le_bytes());
    put(&mut header, 16, id.as_bytes());
    put(&mut header, 48, &SAMPLE_RATE.to_le_bytes());
    put(&mut header, 52, &[1, pools.len() as u8]);
    put(&mut header, 54, &(line as u16).to_le_bytes());
    let crc = crc32fast::hash(&header[..1020]);
    put(&mut header, 1020, &crc.to_le_bytes());
    header.extend(payload);
    Ok(header)
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    use base64::Engine;
    use vibebuddy_firmware_core::adpcm::Decoder;
    use vibebuddy_firmware_core::character_pack;

    use super::*;

    fn line(occasion: &str, text: &str) -> Line {
        Line { occasion: occasion.to_owned(), text: text.to_owned() }
    }

    fn request(voice: &str, lines: Vec<Line>) -> Request {
        Request { id: None, name: "Night Owl".to_owned(), base: "ada".to_owned(), voice: voice.to_owned(), lines }
    }

    /// A line as a TTS answers it: silence, a tone, silence.
    fn spoken() -> Vec<u8> {
        let tone = (0..1600).map(|n| (8000.0 * (n as f64 * 0.2).sin()) as i16);
        std::iter::repeat_n(0i16, 400).chain(tone).chain(std::iter::repeat_n(3, 400)).flat_map(i16::to_le_bytes).collect()
    }

    fn answer(status: &str, content_type: &str, body: &[u8]) -> Vec<u8> {
        let head = format!("HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
        [head.as_bytes(), body].concat()
    }

    /// A fake HTTP server answering the nth request with `respond(n, request)`; returns its base URL and the requests.
    fn serve(respond: impl Fn(usize, &str) -> Vec<u8> + Send + 'static) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = requests.clone();
        std::thread::spawn(move || {
            for socket in listener.incoming() {
                let mut socket = socket.unwrap();
                let mut request = Vec::new();
                let mut buffer = [0u8; 4096];
                loop {
                    let text = String::from_utf8_lossy(&request).into_owned();
                    if let Some((head, body)) = text.split_once("\r\n\r\n") {
                        let length = head.lines().find_map(|line| line.to_ascii_lowercase().strip_prefix("content-length: ")?.parse::<usize>().ok()).unwrap_or(0);
                        if body.len() >= length {
                            break;
                        }
                    }
                    let read = socket.read(&mut buffer).unwrap();
                    request.extend_from_slice(&buffer[..read]);
                }
                let request = String::from_utf8_lossy(&request).into_owned();
                let number = seen.lock().unwrap().len();
                seen.lock().unwrap().push(request.clone());
                socket.write_all(&respond(number, &request)).unwrap();
            }
        });
        (base, requests)
    }

    fn maker(tag: &str, service: Option<String>, keys: &[(Provider, &str)], elevenlabs: &str, volc: &str) -> (Maker, PathBuf) {
        let scratch = crate::voice_keys::tests::scratch(tag);
        let account = Arc::new(crate::account::signed_in_for_tests(&scratch.join("account"), "tok_session"));
        let own = Arc::new(VoiceKeys::new(Some(&scratch)));
        own.set(&keys.iter().map(|(provider, key)| (*provider, Some((*key).to_owned()))).collect::<Vec<_>>()).unwrap();
        let mut maker = Maker::new(Some(scratch.join("characters")), service, account, own);
        (maker.elevenlabs_url, maker.volc_url) = (elevenlabs.to_owned(), volc.to_owned());
        (maker, scratch)
    }

    /// Makes `request` on this thread; answers the status it ended with.
    fn run(maker: &Maker, request: &Request) -> MakingStatus {
        let (id, voice) = maker.begin(request).unwrap();
        maker.make(&id, &voice, request);
        maker.status().unwrap()
    }

    fn text_of(request: &str) -> String {
        let body: serde_json::Value = serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        body["text"].as_str().or(body["req_params"]["text"].as_str()).unwrap().to_owned()
    }

    #[test]
    fn a_form_of_address_goes_with_its_comma_as_in_make_character() {
        assert_eq!(without_address("All done, {address}."), "All done.");
        assert_eq!(without_address("Hey {address}, need you."), "Hey, need you.");
        assert_eq!(without_address("{address}，快来看看。"), "快来看看。");
        assert_eq!(without_address("好了，{address}！"), "好了！");
        assert_eq!(without_address("都这么晚了，{address}快去睡"), "都这么晚了，快去睡");
        assert_eq!(without_address("Nice one {address}"), "Nice one");
    }

    #[test]
    fn lines_with_an_address_are_made_once_per_form_and_named_as_make_character_names_them() {
        let lines = [line("done", "Nice."), line("done", "Done, {address}."), line("failed", "Oops.")];
        let pieces = pieces(&lines, &forms("en"));
        let paths: Vec<String> = pieces.iter().map(|piece| piece.path.display().to_string()).collect();
        // `printf '%s' 'Done, boss.' | shasum -a 1` starts 1641ab442f.
        assert!(paths.contains(&"audio-address/boss/done/0-002-1641ab442f.pcm".to_owned()), "{paths:?}");
        let texts: Vec<&str> = pieces.iter().map(|piece| piece.text.as_str()).collect();
        assert_eq!(texts, ["Nice.", "Done.", "Done, boss.", "Done, captain.", "Done, buddy.", "Oops."]);
        assert!(paths[0].starts_with("audio/done/1-") && paths[1].starts_with("audio/done/0-002-") && paths[3].starts_with("audio-address/captain/done/0-002-"));
        assert_eq!(forms("zh").iter().map(|(form, _)| *form).collect::<Vec<_>>(), ["laoban", "dalao", "ge", "jie", "qin"]);
    }

    #[test]
    fn adpcm_decodes_back_close_to_the_input() {
        let input: Vec<i16> = (0..4001).map(|n| (20000.0 * (n as f64 * 0.05).sin() * (n as f64 * 0.001).cos()) as i16).collect();
        let bytes = encode(&input);
        assert_eq!(bytes.len(), input.len().div_ceil(2));
        let mut decoder = Decoder::default();
        let output: Vec<i16> = bytes.iter().flat_map(|byte| [byte & 15, byte >> 4]).take(input.len()).map(|nibble| decoder.decode(nibble)).collect();
        let error: f64 = input.iter().zip(&output).map(|(a, b)| (f64::from(*a) - f64::from(*b)).powi(2)).sum();
        let signal: f64 = input.iter().map(|a| f64::from(*a).powi(2)).sum();
        assert!(10.0 * (signal / error).log10() > 20.0, "signal to noise {}", 10.0 * (signal / error).log10());
    }

    #[test]
    fn silence_is_trimmed_and_the_peak_is_minus_1_dbfs() {
        let samples = pcm(&spoken());
        let out = trim_and_normalize(&samples).unwrap();
        assert!(out.len() < 1600 && out.len() > 1590, "{}", out.len());
        assert_eq!(out.iter().map(|sample| sample.unsigned_abs()).max(), Some(29205));
        assert_eq!(trim_and_normalize(&[0, 50, -100, 3]), None, "all below -50 dBFS");
    }

    #[test]
    fn the_pools_pack_byte_for_byte_as_character_pack_py_packs_them() {
        // daemon/src/fixtures/character: three lines of PCM, and pack.bin as tools/character_pack.py built it from them
        // (`tools/character_pack.py made-a1b2c3d4 daemon/src/fixtures/character/audio pack.bin`).
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fixtures/character");
        let pools = pools(&fixture.join("audio"));
        assert_eq!(pools.iter().map(Vec::len).collect::<Vec<_>>(), [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
        assert_eq!(pack("made-a1b2c3d4", &pools).unwrap(), std::fs::read(fixture.join("pack.bin")).unwrap());
    }

    #[test]
    fn only_voices_in_voices_tsv_are_accepted() {
        let all = voices();
        // Every preset's voice, and the built-in default's, as the voice service names them.
        for character in [None, Some("ada"), Some("dave"), Some("hank"), Some("kai"), Some("luna"), Some("mei"), Some("wanwanxiaohe")] {
            let voice = crate::synthesis::voice_for(character);
            assert!(all.iter().any(|known| format!("{}:{}", known.provider, known.voice) == voice), "{voice} is missing");
        }
        assert!(all.iter().all(|voice| Provider::named(&voice.provider).is_some() && ["en", "zh"].contains(&voice.language.as_str())));
        let ok = request("luna", vec![line("done", "Done.")]);
        assert_eq!(check(&ok).map(|voice| voice.voice), Ok("ggx46gxUmdtruayPIL8q".to_owned()));
        for (bad, why) in [
            (request("elevenlabs:ggx46gxUmdtruayPIL8q", ok.lines.clone()), "not in voices.tsv"),
            (Request { base: "robot".to_owned(), ..ok.clone() }, "not a preset"),
            (request("luna", vec![line("lunch", "Food.")]), "not an occasion"),
            (request("luna", vec![line("done", "Two\nrows")]), "one row"),
            (request("luna", vec![]), "1 to 110 lines"),
        ] {
            assert!(check(&bad).unwrap_err().contains(why), "{why}");
        }
    }

    #[test]
    fn every_preset_a_character_can_start_from_has_its_lines_and_they_can_be_made() {
        for (id, _) in crate::synthesis::VOICE_FILES {
            let lines = preset_lines(id).unwrap_or_else(|| panic!("{id} has no lines"));
            assert_eq!(lines.len(), 55, "{id}");
            assert!(lines.iter().any(|line| line.text.contains("{address}")), "{id}");
            let voice = voices().into_iter().find(|voice| voice.id == *id).unwrap();
            assert_eq!(check(&Request { id: None, name: "Copy".to_owned(), base: (*id).to_owned(), voice: voice.id, lines }).err(), None, "{id}");
        }
        assert_eq!(preset_lines("ada").unwrap()[0], line("input_required", "Hey {address}, it's waiting on you."));
        assert_eq!(preset_lines("robot"), None);
    }

    #[test]
    fn a_refusal_halfway_stops_the_job_and_starting_again_makes_each_line_once() {
        // The month runs out on the third request, and is back for the next run.
        let refuse = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let refusing = refuse.clone();
        let (service, requests) = serve(move |n, _| match n {
            2 if refusing.load(std::sync::atomic::Ordering::SeqCst) => answer("429 Too Many Requests", "application/json", br#"{"reason":"monthly"}"#),
            _ => answer("200 OK", "audio/pcm;rate=16000", &spoken()),
        });
        let (maker, scratch) = maker("making-resume", Some(service), &[], "http://127.0.0.1:9", "http://127.0.0.1:9");
        let lines = vec![line("done", "All done."), line("done", "Done, {address}."), line("welcome_back", "Back again.")];
        let mut request = request("ada", lines);

        let stopped = run(&maker, &request);
        assert_eq!((stopped.state, stopped.reason, stopped.done, stopped.total), ("failed", Some("monthly"), 2, 6));
        let dir = scratch.join("characters").join(&stopped.id);
        assert!(!dir.join("pack.bin").exists(), "no pack until every line is made");
        let listed = maker.list();
        assert_eq!(listed, [MadeCharacter { id: stopped.id.clone(), name: "Night Owl".to_owned(), base: "ada".to_owned(), voice: "ada".to_owned(), lines: 3, complete: false }]);
        let first = requests.lock().unwrap().clone();
        assert!(first[0].to_ascii_lowercase().contains("authorization: bearer tok_session"), "{}", first[0]);
        assert!(first[0].contains(r#""for":"character""#) && first[0].contains(r#""voice":"elevenlabs:tcSsCJFYrsHpK5rCQZMf""#), "{}", first[0]);

        refuse.store(false, std::sync::atomic::Ordering::SeqCst);
        request.id = Some(stopped.id.clone());
        let done = run(&maker, &request);
        assert_eq!((done.state, done.reason, done.done, done.total), ("done", None, 6, 6));
        assert!(maker.list()[0].complete);
        let all = requests.lock().unwrap().clone();
        // Each line answered with audio once: the refused one was asked again, nothing else was.
        let made: Vec<String> = all.iter().enumerate().filter(|(n, _)| *n != 2).map(|(_, request)| text_of(request)).collect();
        let mut unique = made.clone();
        unique.sort();
        unique.dedup();
        assert_eq!((made.len(), unique.len()), (6, 6), "{made:?}");
        assert_eq!(text_of(&all[2]), text_of(&all[3]), "the refused line is the first made on resuming");

        // The packs read as the box reads them: the address line first in its pool, then the other.
        let pack = std::fs::read(dir.join("pack.bin")).unwrap();
        let parsed = character_pack::parse(&pack, 1 << 21).unwrap();
        assert_eq!(parsed.id(), stopped.id);
        assert_eq!((parsed.pool(Occasion::Done).len(), parsed.pool(Occasion::WelcomeBack).len()), (2, 1));
        for form in ["boss", "captain", "buddy"] {
            let variant = character_pack::parse(&std::fs::read(dir.join(format!("address/{form}.bin"))).unwrap(), 1 << 21).unwrap();
            assert_eq!((variant.pool(Occasion::Done).len(), variant.pool(Occasion::WelcomeBack).len()), (1, 0), "{form}");
        }
        let manifest: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join(MANIFEST)).unwrap()).unwrap();
        assert_eq!((manifest["name"].as_str(), manifest["voice"].as_str(), manifest["lines"].as_array().map(Vec::len)), (Some("Night Owl"), Some("ada"), Some(3)));

        // A line dropped: nothing to synthesize, its audio goes.
        request.lines.pop();
        assert_eq!(run(&maker, &request).state, "done");
        assert_eq!(requests.lock().unwrap().len(), all.len());
        assert!(!dir.join("audio/welcome_back").exists());
        let _ = std::fs::remove_dir_all(scratch);
    }

    #[test]
    fn the_plan_and_a_lost_session_stop_the_job_with_their_reason() {
        let (service, _) = serve(|n, _| match n {
            0 => answer("403 Forbidden", "application/json", br#"{"reason":"plan"}"#),
            1 => answer("429 Too Many Requests", "application/json", br#"{"reason":"global_cap"}"#),
            _ => answer("401 Unauthorized", "application/json", b"{}"),
        });
        let (maker, scratch) = maker("making-refused", Some(service), &[], "http://127.0.0.1:9", "http://127.0.0.1:9");
        let request = request("kai", vec![line("done", "Done.")]);
        for reason in ["plan", "global_cap", "signed_out"] {
            assert_eq!(run(&maker, &request).reason, Some(reason));
        }
        assert!(maker.account.token().is_none(), "a 401 forgets the session");
        let again = run(&maker, &request);
        assert_eq!((again.reason, again.error.as_deref()), (Some("signed_out"), Some("not signed in")));
        let _ = std::fs::remove_dir_all(scratch);
    }

    #[test]
    fn an_own_key_goes_to_its_provider_for_16_khz_pcm() {
        let (eleven, eleven_requests) = serve(|_, _| answer("200 OK", "audio/pcm", &spoken()));
        let objects = format!(
            r#"{{"code":0,"data":"{}"}}{{"code":0,"data":"{}"}}{{"code":20000000,"message":"ok"}}"#,
            base64::engine::general_purpose::STANDARD.encode(&spoken()[..2000]),
            base64::engine::general_purpose::STANDARD.encode(&spoken()[2000..])
        );
        let (volc, volc_requests) = serve(move |_, _| answer("200 OK", "application/json", objects.as_bytes()));
        // The service is on port 9, where nothing answers: a line sent there would fail.
        let keys = [(Provider::ElevenLabs, "sk_own_eleven"), (Provider::Volc, "volc_own")];
        let (maker, scratch) = maker("making-own", Some("http://127.0.0.1:9".to_owned()), &keys, &eleven, &volc);

        assert_eq!(run(&maker, &request("hank", vec![line("done", "Done.")])).state, "done");
        let sent = eleven_requests.lock().unwrap()[0].clone();
        assert!(sent.starts_with("POST /v1/text-to-speech/U6jH23hcPh0zH17nf3DT/stream?output_format=pcm_16000 HTTP/1.1\r\n"), "{sent}");
        assert!(sent.to_ascii_lowercase().contains("xi-api-key: sk_own_eleven") && sent.contains(r#""model_id":"eleven_flash_v2_5""#), "{sent}");
        assert!(!sent.contains("tok_session"));

        let made = run(&maker, &request("wanwanxiaohe", vec![line("done", "好了。")]));
        assert_eq!(made.state, "done", "{made:?}");
        let sent = volc_requests.lock().unwrap()[0].clone();
        assert!(sent.to_ascii_lowercase().contains("x-api-key: volc_own"), "{sent}");
        assert!(sent.contains(r#""format":"pcm""#) && sent.contains(r#""sample_rate":16000"#) && sent.contains(r#""speaker":"zh_female_wanwanxiaohe_moon_bigtts""#), "{sent}");
        let pack = std::fs::read(scratch.join("characters").join(&made.id).join("pack.bin")).unwrap();
        let line = character_pack::parse(&pack, 1 << 21).unwrap().pool(Occasion::Done)[0];
        assert!(line.samples > 1590 && line.samples < 1600, "both pieces of the stream, trimmed: {}", line.samples);
        let _ = std::fs::remove_dir_all(scratch);
    }

    #[test]
    fn a_job_that_panics_ends_as_failed_not_making_forever() {
        let (maker, scratch) = maker("making-panic", None, &[], "http://127.0.0.1:9", "http://127.0.0.1:9");
        let request = request("ada", vec![line("done", "Done.")]);
        maker.begin(&request).unwrap();
        maker.guard(|| {
            let _held = maker.status.lock().unwrap();
            panic!("a bug");
        });
        let status = maker.status().unwrap();
        assert_eq!((status.state, status.reason), ("failed", Some("internal")));
        assert!(maker.begin(&request).is_ok(), "the next one can start");
        let _ = std::fs::remove_dir_all(scratch);
    }

    #[test]
    fn one_character_at_a_time_and_only_one_made_here_is_made_again() {
        let (maker, scratch) = maker("making-busy", None, &[], "http://127.0.0.1:9", "http://127.0.0.1:9");
        let mut request = request("ada", vec![line("done", "Done.")]);
        let (id, _) = maker.begin(&request).unwrap();
        assert!(valid_id(&id) && id.starts_with("night-owl-"), "{id}");
        assert_eq!(maker.begin(&request).unwrap_err(), StartError::Busy);
        request.id = Some("../../elsewhere".to_owned());
        assert!(matches!(maker.begin(&request), Err(StartError::Invalid(_))));
        assert!(new_id("小何").unwrap().starts_with("character-"));
        assert_eq!(new_id("A very long name for a Character indeed").unwrap().len(), 31);
        let _ = std::fs::remove_dir_all(scratch);
    }
}
