//! Who the buddy is when it answers a voice command (ADR-0017): the persona and language of the Character it
//! wears, and the user's form of address, as the resident session's instructions carry them.
//!
//! The box reports only the id of the pack it wears (`VOICES`). Which preset lends a custom Character its voice,
//! persona and lines, and which form of address the pack's lines were written with, only the App knows; it says so
//! when it writes the pack (`POST /v1/device/voice-pack?lender=…&address=…`), and the daemon keeps it in
//! [`RECORD_FILE`].

use std::path::Path;

use serde::{Deserialize, Serialize};

/// What the App said about the pack it last wrote, in the daemon's state directory.
pub const RECORD_FILE: &str = "character.json";
/// The id a custom Character's pack carries, whatever lends it voice and lines.
const CUSTOM: &str = "custom";
/// The default Character, and the persona of any pack these files don't know.
const ROBOT: &str = "robot";
/// The robot's lines are the built-in voice's, in English; it has no `voice.env`.
const ROBOT_LANGUAGE: &str = "en";

/// Each Character's persona (`characters/<id>/persona.md`), the same one its lines were written in.
const PERSONAS: &[(&str, &str)] = &[
    ("ada", include_str!("../../characters/ada/persona.md")),
    ("dave", include_str!("../../characters/dave/persona.md")),
    ("hank", include_str!("../../characters/hank/persona.md")),
    ("kai", include_str!("../../characters/kai/persona.md")),
    ("luna", include_str!("../../characters/luna/persona.md")),
    ("mei", include_str!("../../characters/mei/persona.md")),
    ("robot", include_str!("../../characters/robot/persona.md")),
    ("wanwanxiaohe", include_str!("../../characters/wanwanxiaohe/persona.md")),
];

/// The forms of address the App offers (docs/characters.md): id, words, language.
const FORMS_OF_ADDRESS: &[(&str, &str, &str)] = &[
    ("laoban", "老板", "zh"),
    ("dalao", "大佬", "zh"),
    ("ge", "哥", "zh"),
    ("jie", "姐", "zh"),
    ("qin", "亲", "zh"),
    ("boss", "boss", "en"),
    ("captain", "captain", "en"),
    ("buddy", "buddy", "en"),
];

/// What the App said when it last wrote a Character pack: its id, the preset lending a custom Character its
/// voice, persona and lines, and the form of address its lines carry.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(default)]
pub struct Written {
    pub id: Option<String>,
    pub lender: Option<String>,
    pub address: Option<String>,
}

impl Written {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self).map_err(std::io::Error::other)?)
    }
}

/// The Character the buddy wears, as far as its voice and persona go.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Wearing {
    /// Whose voice and persona: the lender for a custom Character; None with no Character pack.
    pub character: Option<String>,
    /// The form of address the user picked (`boss`, `laoban`), if any.
    pub address: Option<String>,
}

impl Wearing {
    /// The Character pack `id` (the box's report, else the one last written), with what the App said when it
    /// wrote it, if that was this pack.
    pub fn of(id: Option<String>, written: &Written) -> Self {
        match id {
            Some(id) if written.id.as_deref() == Some(id.as_str()) => Self {
                character: if id == CUSTOM { written.lender.clone() } else { Some(id) },
                address: written.address.clone(),
            },
            id => Self { character: id, address: None },
        }
    }
}

/// How the buddy speaks its spoken replies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Persona {
    /// The Character whose persona this is.
    pub id: &'static str,
    /// Its name, from the persona's heading.
    pub name: &'static str,
    /// The persona, without its heading.
    pub text: &'static str,
    /// The one language it speaks, in English (`English`, `Chinese`).
    pub language: &'static str,
    /// The words of the user's form of address, when one in this language is picked.
    pub address: Option<&'static str>,
}

impl Persona {
    /// The persona of what the buddy wears: the robot's for no pack, a custom Character with no known lender,
    /// or a pack these files don't know.
    pub fn of(wearing: &Wearing) -> Self {
        let (id, file) = wearing
            .character
            .as_deref()
            .and_then(|character| PERSONAS.iter().find(|(id, _)| *id == character))
            .or_else(|| PERSONAS.iter().find(|(id, _)| *id == ROBOT))
            .copied()
            .expect("the robot has a persona");
        let (heading, text) = file.split_once('\n').unwrap_or((file, ""));
        let code = crate::synthesis::VOICE_FILES
            .iter()
            .find(|(voice, _)| *voice == id)
            .and_then(|(_, voice)| crate::synthesis::voice_field(voice, "LANGUAGE"))
            .unwrap_or(ROBOT_LANGUAGE);
        let address = wearing.address.as_deref().and_then(|form| {
            FORMS_OF_ADDRESS.iter().find(|(id, _, language)| *id == form && *language == code).map(|(_, words, _)| *words)
        });
        Self {
            id,
            name: heading.trim_start_matches('#').trim(),
            text: text.trim(),
            language: match code {
                "zh" => "Chinese",
                _ => "English",
            },
            address,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wearing(character: Option<&str>, address: Option<&str>) -> Wearing {
        Wearing { character: character.map(str::to_owned), address: address.map(str::to_owned) }
    }

    #[test]
    fn each_character_has_its_own_persona_name_and_language() {
        let ada = Persona::of(&wearing(Some("ada"), None));
        assert_eq!((ada.id, ada.name, ada.language), ("ada", "Ada", "English"));
        assert!(ada.text.starts_with("Speaks English") && ada.text.contains("Londoner"), "{}", ada.text);
        let xiaohe = Persona::of(&wearing(Some("wanwanxiaohe"), None));
        assert_eq!((xiaohe.id, xiaohe.name, xiaohe.language), ("wanwanxiaohe", "Xiaohe (小何)", "Chinese"));
        for id in ["dave", "hank", "kai", "luna", "mei"] {
            let persona = Persona::of(&wearing(Some(id), None));
            assert_eq!((persona.id, persona.language), (id, "English"));
            assert!(!persona.text.is_empty() && !persona.text.starts_with('#'));
        }
    }

    #[test]
    fn no_pack_or_an_unknown_one_is_the_robot() {
        for character in [None, Some("robot"), Some("custom"), Some("hsiaoyu")] {
            let persona = Persona::of(&wearing(character, None));
            assert_eq!((persona.id, persona.name, persona.language), ("robot", "VibeBuddy", "English"), "{character:?}");
        }
    }

    #[test]
    fn a_form_of_address_counts_only_in_the_characters_language() {
        assert_eq!(Persona::of(&wearing(Some("ada"), Some("boss"))).address, Some("boss"));
        assert_eq!(Persona::of(&wearing(Some("wanwanxiaohe"), Some("laoban"))).address, Some("老板"));
        assert_eq!(Persona::of(&wearing(Some("ada"), Some("laoban"))).address, None);
        assert_eq!(Persona::of(&wearing(Some("ada"), Some("darling"))).address, None);
    }

    #[test]
    fn a_custom_character_wears_its_lenders_voice_and_persona_and_the_form_written_with_it() {
        let written = Written { id: Some("custom".to_owned()), lender: Some("ada".to_owned()), address: Some("boss".to_owned()) };
        assert_eq!(Wearing::of(Some("custom".to_owned()), &written), wearing(Some("ada"), Some("boss")));
        let persona = Persona::of(&Wearing::of(Some("custom".to_owned()), &written));
        assert_eq!((persona.id, persona.address), ("ada", Some("boss")));
        // The box reports another pack than the App last wrote (written elsewhere): only its id is known.
        assert_eq!(Wearing::of(Some("mei".to_owned()), &written), wearing(Some("mei"), None));
        // A custom Character written by an App that didn't say who lends it: the robot's persona.
        assert_eq!(Wearing::of(Some("custom".to_owned()), &Written::default()), wearing(Some("custom"), None));
        assert_eq!(Wearing::of(None, &written), wearing(None, None));
    }

    #[test]
    fn the_record_round_trips_and_a_missing_one_is_empty() {
        let dir = std::env::temp_dir().join(format!("vibebuddy-persona-{}", std::process::id()));
        let path = dir.join(RECORD_FILE);
        assert_eq!(Written::load(&path), Written::default());
        let written = Written { id: Some("custom".to_owned()), lender: Some("kai".to_owned()), address: None };
        written.save(&path).unwrap();
        assert_eq!(Written::load(&path), written);
        let _ = std::fs::remove_dir_all(dir);
    }
}
