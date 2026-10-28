//! Making a Character (ADR-0020, docs/characters.md), the Mac app's `Making.swift`: the user starts from a preset,
//! edits its lines and picks a voice; the daemon makes the line audio (`POST /v1/characters`) into
//! `config_dir()/characters/<id>/`, and the app packs it with a look and writes it to the box like a preset.

use serde::{Deserialize, Serialize};

use crate::i18n::tr;
use crate::status::{Account, VoiceKeys};

/// A voice a made Character can speak with, from `GET /v1/voices`: a row of characters/voices.tsv, or one in the
/// user's own account (`own`), which only their own key speaks and which has no language of its own.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Voice {
    pub id: String,
    /// `elevenlabs` or `volc`.
    pub provider: String,
    pub language: String,
    pub name: String,
    pub own: bool,
}

impl Voice {
    /// The provider as the UI names it.
    pub fn provider_name(&self) -> String {
        if self.provider == "volc" { tr("Volcano Engine", &[]) } else { "ElevenLabs".to_owned() }
    }
}

impl std::fmt::Display for Voice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} · {}", self.name, self.provider_name())
    }
}

/// The Character whose preview is said in voice `id`: voices.tsv lists each preset's voice under the preset's id, and
/// Jessica's is the robot's, the built-in lines.
pub fn preview_of(id: &str) -> &str {
    if id == "jessica" { "robot" } else { id }
}

/// Voices in `language` first, then the rest of ours, then the user's own; the daemon's order otherwise kept.
pub fn sorted(voices: &[Voice], language: Option<&str>) -> Vec<Voice> {
    let (own, ours): (Vec<Voice>, Vec<Voice>) = voices.iter().cloned().partition(|voice| voice.own);
    let (mut first, rest): (Vec<Voice>, Vec<Voice>) = ours.into_iter().partition(|voice| Some(voice.language.as_str()) == language);
    first.extend(rest);
    first.extend(own);
    first
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct Line {
    pub occasion: String,
    pub text: String,
}

/// `POST /v1/characters`, and a made Character's manifest.json, which has no `id`.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct Request {
    /// A Character made before, to make again (resume); None for a new one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    /// The preset it started from.
    pub base: String,
    /// An id from `GET /v1/voices`.
    pub voice: String,
    pub lines: Vec<Line>,
}

/// A Character made on this computer, from `GET /v1/characters`.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct MadeCharacter {
    pub id: String,
    pub name: String,
    pub base: String,
    pub voice: String,
    /// How many lines were asked for.
    pub lines: usize,
    /// Its packs are made, so it can be written to the box.
    pub complete: bool,
}

/// `making` in the status: the Character being made, or the last one, since the daemon started.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Making {
    pub id: String,
    pub name: String,
    /// `making`, `done` or `failed`; kept as text so a state added later doesn't break the whole status.
    pub state: String,
    /// Pieces of audio made, of all there are.
    pub done: usize,
    pub total: usize,
    /// Why a failed job stopped: `plan`, `monthly`, `global_cap`, `signed_out`, `own_key`, `internal`, or None with `error`.
    pub reason: Option<String>,
    pub error: Option<String>,
}

impl Making {
    pub fn running(&self) -> bool {
        self.state == "making"
    }

    pub fn progress(&self) -> f32 {
        if self.total == 0 { 0.0 } else { self.done as f32 / self.total as f32 }
    }

    /// Why it stopped, said for the user; `provider` names the voice's provider. None unless it failed.
    pub fn explanation(&self, provider: &str) -> Option<String> {
        if self.state != "failed" {
            return None;
        }
        Some(match self.reason.as_deref() {
            Some("plan") => tr("Making a Character with VibeBuddy's voices needs the Paid plan, or your own key for %@.", &[&provider]),
            Some("monthly") => tr("This month's Characters on the Paid plan are used up. The lines made so far are kept: resume next month, or with your own key for %@.", &[&provider]),
            Some("global_cap") => tr("VibeBuddy's voices are paused for today. The lines made so far are kept: resume tomorrow.", &[]),
            Some("signed_out") => tr("Sign in on the Account tab, or set your own key for %@ there, then resume.", &[&provider]),
            Some("own_key") => tr("%@ refused your own key. Set another one on the Account tab, then resume.", &[&provider]),
            Some("internal") => tr("Making the Character stopped unexpectedly. Resume to try again.", &[]),
            _ => tr("Making the Character failed: %@", &[&self.error.as_deref().unwrap_or_default()]),
        })
    }
}

/// Whether a Character can be made in a voice, before starting: the daemon only finds out about the plan once the
/// voice service answers, so the app says what it can tell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Readiness {
    /// The user's own key for the voice's provider: nothing else is needed.
    OwnKey,
    /// Signed in without an own key: VibeBuddy's voices, which need the Paid plan.
    PaidPlan,
    /// Not signed in and no own key.
    SignIn,
    /// No service to sign in to in this build, and no own key.
    OwnKeyOnly,
}

impl Readiness {
    pub fn of(voice: &Voice, keys: Option<&VoiceKeys>, account: &Account) -> Readiness {
        let key = keys.map(|keys| if voice.provider == "volc" { &keys.volc } else { &keys.elevenlabs });
        if key.is_some_and(|key| key.set && !key.refused) {
            Readiness::OwnKey
        } else if voice.own {
            // A voice in the user's own account: VibeBuddy's voice service can't speak it.
            Readiness::OwnKeyOnly
        } else if account.signed_in {
            Readiness::PaidPlan
        } else if account.available {
            Readiness::SignIn
        } else {
            Readiness::OwnKeyOnly
        }
    }

    pub fn can_start(self) -> bool {
        matches!(self, Readiness::OwnKey | Readiness::PaidPlan)
    }

    pub fn explanation(self, provider: &str) -> String {
        match self {
            Readiness::OwnKey => tr("Made with your own key for %@, billed to your account there.", &[&provider]),
            Readiness::PaidPlan => tr("Without your own key for %@, making a Character with VibeBuddy's voices needs the Paid plan.", &[&provider]),
            Readiness::SignIn => tr("Sign in on the Account tab for the Paid plan, or set your own key for %@ there.", &[&provider]),
            Readiness::OwnKeyOnly => tr("Set your own key for %@ on the Account tab first.", &[&provider]),
        }
    }
}

/// What a pack with a look has room for, and the daemon's limits.
const MAX_LINES: usize = 110;
const MAX_TEXT: usize = 1000;
const MAX_NAME: usize = 64;

/// Whether line `index` can go: not when it is the last of its occasion.
pub fn can_remove(index: usize, lines: &[Line]) -> bool {
    lines.get(index).is_some_and(|line| lines.iter().filter(|other| other.occasion == line.occasion).count() > 1)
}

/// What stops these from being made, said for the user; None when they can be. `occasions` are the preset's.
pub fn problem(name: &str, lines: &[Line], occasions: &[String]) -> Option<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME {
        return Some(tr("Give the Character a name of up to 64 characters.", &[]));
    }
    if lines.len() > MAX_LINES {
        return Some(tr("A Character has at most 110 lines.", &[]));
    }
    if let Some(occasion) = occasions.iter().find(|occasion| !lines.iter().any(|line| &line.occasion == *occasion)) {
        return Some(tr("Keep at least one line for “%@”.", &[&occasion_name(occasion)]));
    }
    for line in lines {
        if line.text.trim().is_empty() {
            return Some(tr("A line for “%@” is empty.", &[&occasion_name(&line.occasion)]));
        }
        if line.text.encode_utf16().count() > MAX_TEXT || line.text.contains(['\n', '\r', '\t']) {
            return Some(tr("Each line is one row of at most 1000 characters.", &[]));
        }
        if line.text.replace("{address}", "").contains(['{', '}']) {
            return Some(tr("Only {address} goes in braces: it is where the form of address is said.", &[]));
        }
    }
    None
}

/// The occasions a line is said on (docs/characters.md), as the line editor names them.
pub fn occasion_name(id: &str) -> String {
    let key = match id {
        "input_required" => "Needs input",
        "done" => "Done",
        "failed" => "Failed",
        "focus_done" => "Focus ended",
        "break_done" => "Break ended",
        "first_done" => "First done of the day",
        "milestone" => "Milestone",
        "late_night_done" => "Done late at night",
        "late_night_input" => "Needs input late at night",
        "greeting_morning" => "Morning greeting",
        "greeting_afternoon" => "Afternoon greeting",
        "greeting_evening" => "Evening greeting",
        "long_session" => "Long session",
        "welcome_back" => "Welcome back",
        _ => return id.to_owned(),
    };
    tr(key, &[])
}

/// Where the daemon keeps a Character made here: its packs, its manifest, and the look the user brought for it, which
/// the app keeps there as `look.bin`.
pub fn directory(id: &str) -> Option<std::path::PathBuf> {
    crate::config_dir().map(|dir| dir.join("characters").join(id))
}

/// The look a made Character wears: the one the user brought for it, else its base preset's.
pub async fn look_of(made: &MadeCharacter) -> Option<Vec<u8>> {
    if let Some(look) = directory(&made.id).and_then(|dir| std::fs::read(dir.join("look.bin")).ok()) {
        return Some(look);
    }
    crate::character::Pack::parse(&crate::assets::read_pack(&made.base).await.ok()?)?.look
}

/// A made Character as the box gets it, said with `form` when its packs have that form of address. Works on any
/// plan: nothing is synthesized again.
pub async fn pack_of(made: &MadeCharacter, form: Option<&str>) -> Result<Vec<u8>, String> {
    let dir = directory(&made.id).ok_or("no config directory")?;
    let pack = tokio::fs::read(dir.join("pack.bin")).await.map_err(|_| tr("“%@” isn't finished yet.", &[&made.name]))?;
    let variant = match form {
        Some(form) => tokio::fs::read(dir.join("address").join(format!("{form}.bin"))).await.ok(),
        None => None,
    };
    crate::character::made(&pack, variant.as_deref(), look_of(made).await, &made.id).ok_or_else(|| tr("“%@” isn't finished yet.", &[&made.name]))
}

/// What was asked for a made Character, to make the rest of it after its job stopped.
pub fn manifest(id: &str) -> Result<Request, String> {
    let path = directory(id).ok_or("no config directory")?.join("manifest.json");
    let text = std::fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut request: Request = serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))?;
    request.id = Some(id.to_owned());
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::VoiceKey;

    fn line(occasion: &str, text: &str) -> Line {
        Line { occasion: occasion.to_owned(), text: text.to_owned() }
    }

    #[test]
    fn edited_lines_keep_a_line_per_occasion_and_their_form_of_address() {
        let lines = vec![line("done", "Done, {address}."), line("done", "Nice."), line("failed", "Oops.")];
        let occasions = ["done".to_owned(), "failed".to_owned()];
        assert_eq!(problem("Owl", &lines, &occasions), None);
        assert!(can_remove(0, &lines) && !can_remove(2, &lines) && !can_remove(3, &lines));
        assert!(problem("Owl", &lines[..2], &occasions).is_some(), "an occasion without a line");
        assert!(problem("  ", &lines, &occasions).is_some(), "no name");
        assert!(problem(&"x".repeat(65), &lines, &occasions).is_some(), "a name too long");
        for bad in [" ", "Two\nrows", "Hey {adress}.", &"x".repeat(1001)] {
            let edited: Vec<Line> = std::iter::once(line("done", bad)).chain(lines.iter().cloned()).collect();
            assert!(problem("Owl", &edited, &occasions).is_some(), "{bad:.20}");
        }
        let many: Vec<Line> = std::iter::repeat_n(lines[1].clone(), 110).chain([lines[2].clone()]).collect();
        assert!(problem("Owl", &many, &occasions).is_some(), "too many lines");
    }

    #[test]
    fn without_an_own_key_for_its_provider_a_voice_needs_the_paid_plan() {
        let voice = |provider: &str| Voice { id: "v".to_owned(), provider: provider.to_owned(), language: "en".to_owned(), name: "V".to_owned(), own: false };
        let keys = VoiceKeys { elevenlabs: VoiceKey { set: true, refused: false }, volc: VoiceKey { set: true, refused: true } };
        let signed_in = Account { available: true, signed_in: true, ..Account::default() };
        let signed_out = Account { available: true, ..Account::default() };
        assert_eq!(Readiness::of(&voice("elevenlabs"), Some(&keys), &Account::default()), Readiness::OwnKey);
        assert_eq!(Readiness::of(&voice("volc"), Some(&keys), &signed_in), Readiness::PaidPlan, "a refused key doesn't count");
        assert_eq!(Readiness::of(&voice("volc"), None, &signed_out), Readiness::SignIn);
        assert_eq!(Readiness::of(&voice("volc"), None, &Account::default()), Readiness::OwnKeyOnly);
        assert!(!Readiness::SignIn.can_start() && Readiness::PaidPlan.can_start());
        // A voice of the user's own: only their own key speaks it, whatever the plan.
        let own = Voice { own: true, ..voice("volc") };
        let signed_in = Account { available: true, signed_in: true, ..Account::default() };
        assert_eq!(Readiness::of(&own, None, &signed_in), Readiness::OwnKeyOnly);
    }

    #[test]
    fn a_voice_of_the_users_own_reads_and_comes_after_ours() {
        let voices: Vec<Voice> = serde_json::from_str(r#"[{"id":"own-1a2b3c4d","provider":"volc","voice":"S_me","language":"","gender":"","name":"Me","own":true},{"id":"ada","provider":"elevenlabs","voice":"t","language":"en","gender":"female","name":"Ada"}]"#).unwrap();
        assert!(voices[0].own && !voices[1].own, "a voice without own is ours");
        assert_eq!(sorted(&voices, None).iter().map(|voice| voice.id.as_str()).collect::<Vec<_>>(), ["ada", "own-1a2b3c4d"]);
    }

    #[test]
    fn the_daemons_answers_read_and_a_new_character_is_sent_without_an_id() {
        let made: Vec<MadeCharacter> = serde_json::from_str(r#"[{"id":"owl-1a2b3c4d","name":"Owl","base":"ada","voice":"luna","lines":55,"complete":false}]"#).unwrap();
        assert_eq!(made[0], MadeCharacter { id: "owl-1a2b3c4d".to_owned(), name: "Owl".to_owned(), base: "ada".to_owned(), voice: "luna".to_owned(), lines: 55, complete: false });
        let voices: Vec<Voice> = serde_json::from_str(r#"[{"id":"ada","provider":"elevenlabs","voice":"t","language":"en","gender":"female","name":"Ada"},{"id":"wanwanxiaohe","provider":"volc","voice":"z","language":"zh","gender":"female","name":"Xiaohe"}]"#).unwrap();
        assert_eq!(sorted(&voices, Some("zh")).iter().map(|voice| voice.id.as_str()).collect::<Vec<_>>(), ["wanwanxiaohe", "ada"]);
        let request = Request { id: None, name: "Owl".to_owned(), base: "ada".to_owned(), voice: "luna".to_owned(), lines: vec![line("done", "Done.")] };
        let sent = serde_json::to_string(&request).unwrap();
        assert!(!sent.contains("\"id\"") && sent.contains(r#""occasion":"done""#), "{sent}");
        let manifest: Request = serde_json::from_str(r#"{"name":"Owl","base":"ada","voice":"luna","lines":[{"occasion":"done","text":"Done."}]}"#).unwrap();
        assert_eq!(manifest, request);
        let failed = Making { state: "failed".to_owned(), reason: Some("plan".to_owned()), done: 2, total: 6, ..Making::default() };
        // In whichever language the UI is, it names the provider.
        assert!(!failed.running() && failed.explanation("ElevenLabs").unwrap().contains("ElevenLabs"));
        assert_eq!(Making { state: "done".to_owned(), ..Making::default() }.explanation("ElevenLabs"), None);
    }
}
