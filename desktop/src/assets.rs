//! What was installed next to the app: the Character packs from `characters/`. Firmware isn't installed; the daemon
//! downloads it (ADR-0010). On the Mac the packs ride inside the app bundle. Here `install.sh` puts them in `~/.local/share/vibebuddy`
//! and a distribution package in `/usr/share/vibebuddy`; the user's copy wins, as XDG data lookups go.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::i18n::{self, tr};

/// Where to look, in order: `$XDG_DATA_HOME`, then each of `$XDG_DATA_DIRS`.
fn data_dirs() -> Vec<PathBuf> {
    search_dirs(std::env::var_os("XDG_DATA_HOME"), std::env::var_os("HOME"), std::env::var_os("XDG_DATA_DIRS"))
}

fn search_dirs(data_home: Option<OsString>, home: Option<OsString>, data_dirs: Option<OsString>) -> Vec<PathBuf> {
    let home_dir = data_home
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| home.map(|home| PathBuf::from(home).join(".local/share")));
    // The spec's default when the variable is unset or empty.
    let system = data_dirs.filter(|dirs| !dirs.is_empty()).unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    home_dir
        .into_iter()
        .chain(std::env::split_paths(&system).filter(|dir| dir.is_absolute()))
        .map(|dir| dir.join("vibebuddy"))
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Language {
    Chinese,
    English,
}

impl Language {
    pub const ALL: [Language; 2] = [Language::Chinese, Language::English];

    /// As in file and setting names, the Mac's `VoiceLanguage` raw value.
    pub fn code(self) -> &'static str {
        match self {
            Language::Chinese => "zh",
            Language::English => "en",
        }
    }
}

/// What the buddy can call the user, the Mac's `FormOfAddress.all` and characters/addresses.tsv.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormOfAddress {
    pub id: &'static str,
    /// The words spoken, shown as they are in every UI language.
    pub words: &'static str,
    pub language: Language,
}

pub const FORMS_OF_ADDRESS: [FormOfAddress; 8] = [
    FormOfAddress { id: "laoban", words: "老板", language: Language::Chinese },
    FormOfAddress { id: "dalao", words: "大佬", language: Language::Chinese },
    FormOfAddress { id: "ge", words: "哥", language: Language::Chinese },
    FormOfAddress { id: "jie", words: "姐", language: Language::Chinese },
    FormOfAddress { id: "qin", words: "亲", language: Language::Chinese },
    FormOfAddress { id: "boss", words: "boss", language: Language::English },
    FormOfAddress { id: "captain", words: "captain", language: Language::English },
    FormOfAddress { id: "buddy", words: "buddy", language: Language::English },
];

/// The Mac app's `VoiceCatalogEntry.all`, with the same English keys for names and tags.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Voice {
    pub id: &'static str,
    pub name: String,
    pub tag: String,
    /// Who the Character is, in one sentence, so the user can pick between them.
    pub summary: String,
    pub language: Language,
    /// The normal frame of the Character's look, for its card; None for a pack without one.
    pub face: Option<crate::character::Image>,
}

fn catalog() -> Vec<Voice> {
    let voice = |id, name: String, tag: String, summary: &str, language| Voice {
        id,
        name,
        tag,
        summary: tr(summary, &[]),
        language,
        face: None,
    };
    vec![
        voice(
            "wanwanxiaohe",
            tr("Xiaohe", &[]),
            tr("Chinese · Taiwanese accent", &[]),
            "A sweet, lively friend from Taiwan who's always rooting for you.",
            Language::Chinese,
        ),
        voice(
            "ahu",
            tr("Ahu", &[]),
            tr("Chinese · Beijing accent", &[]),
            "A Beijing guy with the gift of the gab. Hearty praise, and nothing fazes him.",
            Language::Chinese,
        ),
        voice(
            "amanda",
            "Amanda".to_owned(),
            tr("English", &[]),
            "A cheerful friend who celebrates every small win.",
            Language::English,
        ),
        voice(
            "jackson",
            "Jackson".to_owned(),
            tr("English", &[]),
            "The calm coworker at the next desk. Dry humor, rare but honest praise.",
            Language::English,
        ),
        voice(
            "ada",
            "Ada".to_owned(),
            tr("English · British accent", &[]),
            "A witty Londoner. Understated praise, gently bossy about breaks.",
            Language::English,
        ),
        voice(
            "luna",
            "Luna".to_owned(),
            tr("English", &[]),
            "Late-night lofi calm. Never rushes you.",
            Language::English,
        ),
        voice(
            "mei",
            "Mei".to_owned(),
            tr("English · Bay Area", &[]),
            "An upbeat Bay Area engineer who cheers you on, sometimes in Chinese.",
            Language::English,
        ),
    ]
}

/// The voices whose pack was installed, those in the UI's language first.
pub fn voices() -> Vec<Voice> {
    let preferred = if i18n::is_chinese() { Language::Chinese } else { Language::English };
    let mut voices: Vec<Voice> = catalog()
        .into_iter()
        .filter_map(|mut voice| {
            let pack = std::fs::read(find_voice_pack(voice.id)?).ok();
            voice.face = pack
                .as_deref()
                .and_then(crate::character::Pack::parse)
                .and_then(|pack| crate::character::look_frames(pack.look.as_deref()?))
                .and_then(|frames| frames.into_iter().next());
            Some(voice)
        })
        .collect();
    voices.sort_by_key(|voice| voice.language != preferred);
    voices
}

/// `id` as the catalog's own string, if the catalog has it.
pub fn voice_id(id: &str) -> Option<&'static str> {
    catalog().into_iter().find(|voice| voice.id == id).map(|voice| voice.id)
}

pub fn voice_name(id: &str) -> String {
    if id == "builtin" {
        return tr("Built-in voice (Jessica)", &[]);
    }
    if id == "custom" {
        return tr("Your own character", &[]);
    }
    if id == "robot" {
        return "Vibe Buddy".to_owned();
    }
    catalog().into_iter().find(|voice| voice.id == id).map(|voice| voice.name).unwrap_or_else(|| id.to_owned())
}

/// The voice to offer when the UI switches language, as the Mac's `switchSuggestion`: none when the box already
/// speaks `language` or its voice is unknown ("builtin" is Jessica, English); otherwise the first installed voice in
/// `language`, in catalog order.
pub fn voice_switch(box_voice: &str, language: Language) -> Option<Voice> {
    switch_among(catalog(), box_voice, language, |id| find_voice_pack(id).is_some())
}

fn switch_among(catalog: Vec<Voice>, box_voice: &str, language: Language, installed: impl Fn(&str) -> bool) -> Option<Voice> {
    let current = if box_voice == "builtin" {
        Language::English
    } else {
        catalog.iter().find(|voice| voice.id == box_voice)?.language
    };
    if current == language {
        return None;
    }
    catalog.into_iter().find(|voice| voice.language == language && installed(voice.id))
}

fn voice_pack(dir: &Path, id: &str) -> PathBuf {
    dir.join("voices").join(format!("{id}.bin"))
}

fn find_voice_pack(id: &str) -> Option<PathBuf> {
    data_dirs().into_iter().map(|dir| voice_pack(&dir, id)).find(|path| path.is_file())
}

/// An installed pack as it is, such as the robot with the built-in voice's lines (voices/robot.bin).
pub async fn read_pack(id: &str) -> Result<Vec<u8>, String> {
    let path = find_voice_pack(id).ok_or_else(|| format!("no voice pack named {id} is installed"))?;
    tokio::fs::read(&path).await.map_err(|error| format!("{}: {error}", path.display()))
}

/// The language a box Character speaks, from the id the box reports; None for one the catalog doesn't know.
pub fn language_of(id: &str) -> Option<Language> {
    catalog().into_iter().find(|voice| voice.id == id).map(|voice| voice.language)
}

/// Character `id` as the box should get it: its pack, with the lines of the form of address picked for
/// its language swapped in when one is (installed as voices/<id>.<form>.bin next to the pack).
pub async fn read_character(id: &str, form: Option<&str>) -> Result<crate::character::Pack, String> {
    let path = find_voice_pack(id).ok_or_else(|| format!("no voice pack named {id} is installed"))?;
    let data = tokio::fs::read(&path).await.map_err(|error| format!("{}: {error}", path.display()))?;
    let pack = crate::character::Pack::parse(&data).ok_or_else(|| format!("{} is not a Character pack", path.display()))?;
    let variant = form.and_then(|form| {
        data_dirs().into_iter().map(|dir| dir.join("voices").join(format!("{id}.{form}.bin"))).find(|path| path.is_file())
    });
    Ok(match variant.and_then(|path| std::fs::read(path).ok()).as_deref().and_then(crate::character::Pack::parse) {
        Some(variant) => pack.with_address(&variant),
        None => pack,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_users_copy_comes_before_the_systems() {
        let dirs = search_dirs(None, Some("/home/me".into()), Some("/usr/local/share:/usr/share".into()));
        assert_eq!(
            dirs,
            ["/home/me/.local/share/vibebuddy", "/usr/local/share/vibebuddy", "/usr/share/vibebuddy"].map(PathBuf::from)
        );
    }

    #[test]
    fn unset_or_empty_data_dirs_fall_back_to_the_spec_default() {
        let expected = ["/xdg/vibebuddy", "/usr/local/share/vibebuddy", "/usr/share/vibebuddy"].map(PathBuf::from);
        assert_eq!(search_dirs(Some("/xdg".into()), None, None), expected);
        assert_eq!(search_dirs(Some("/xdg".into()), None, Some("".into())), expected);
    }

    #[test]
    fn a_voice_switch_is_offered_only_across_languages() {
        let all = |_: &str| true;
        let switch = |voice, language, installed: &dyn Fn(&str) -> bool| {
            switch_among(catalog(), voice, language, installed).map(|voice| voice.id)
        };
        assert_eq!(switch("builtin", Language::Chinese, &all), Some("wanwanxiaohe"));
        assert_eq!(switch("wanwanxiaohe", Language::English, &all), Some("amanda"));
        assert_eq!(switch("jackson", Language::English, &all), None);
        assert_eq!(switch("someone-else", Language::English, &all), None);
        assert_eq!(switch("builtin", Language::Chinese, &|id: &str| id == "ahu"), Some("ahu"));
        assert_eq!(switch("builtin", Language::Chinese, &|_: &str| false), None);
    }

    #[test]
    fn relative_entries_are_ignored() {
        let dirs = search_dirs(Some("relative".into()), None, Some("share:/opt/share".into()));
        assert_eq!(dirs, [PathBuf::from("/opt/share/vibebuddy")]);
    }
}
