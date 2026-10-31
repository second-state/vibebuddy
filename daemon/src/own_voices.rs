//! The user's own voices (docs/characters.md, "Your own voice"): a voice in the user's own ElevenLabs or Volcano
//! Engine account, cloned or designed there, which a made Character can speak with through the user's own key for
//! that provider. Kept in `own-voices.json` in the config directory: where the voice is (the provider, its id there,
//! the Volcano Engine resource it answered on) and the name the user gave it, never audio or a key.
//! [`crate::making`] tries a voice before it is kept.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tracing::warn;

pub const FILE: &str = "own-voices.json";

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct OwnVoice {
    /// `own-` and eight hex digits, as a made Character's manifest names it.
    pub id: String,
    /// `elevenlabs` or `volc`.
    pub provider: String,
    /// The provider's id for it: an ElevenLabs voice id, a Volcano Engine speaker.
    pub voice: String,
    pub name: String,
    /// The Volcano Engine resource it answered on (`seed-icl-2.0` for a voice cloned with 2.0, and so on); None for
    /// ElevenLabs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
}

pub struct OwnVoices {
    /// None without a config directory, and in tests unless one sets it; then none can be kept.
    file: Option<PathBuf>,
    voices: Mutex<Vec<OwnVoice>>,
}

impl OwnVoices {
    pub fn new(config_dir: Option<&Path>) -> Self {
        let file = config_dir.map(|dir| dir.join(FILE));
        let voices = file.as_deref().map(load).unwrap_or_default();
        Self { file, voices: Mutex::new(voices) }
    }

    pub fn list(&self) -> Vec<OwnVoice> {
        self.voices.lock().unwrap().clone()
    }

    pub fn get(&self, id: &str) -> Option<OwnVoice> {
        self.voices.lock().unwrap().iter().find(|voice| voice.id == id).cloned()
    }

    /// Keeps a voice, or renames the one already kept for the same provider and id; answers it as kept.
    pub fn add(&self, provider: &str, voice: &str, name: &str, resource: Option<String>) -> std::io::Result<OwnVoice> {
        let mut voices = self.voices.lock().unwrap();
        let mut next = voices.clone();
        let kept = match next.iter_mut().find(|kept| kept.provider == provider && kept.voice == voice) {
            Some(kept) => {
                kept.name = name.to_owned();
                kept.resource = resource;
                kept.clone()
            }
            None => {
                let mut bytes = [0u8; 4];
                getrandom::fill(&mut bytes).map_err(std::io::Error::other)?;
                let kept = OwnVoice {
                    id: format!("own-{:08x}", u32::from_le_bytes(bytes)),
                    provider: provider.to_owned(),
                    voice: voice.to_owned(),
                    name: name.to_owned(),
                    resource,
                };
                next.push(kept.clone());
                kept
            }
        };
        self.save(&next)?;
        *voices = next;
        Ok(kept)
    }

    /// Forgets a voice; false when none of that id is kept.
    pub fn remove(&self, id: &str) -> std::io::Result<bool> {
        let mut voices = self.voices.lock().unwrap();
        let next: Vec<OwnVoice> = voices.iter().filter(|voice| voice.id != id).cloned().collect();
        if next.len() == voices.len() {
            return Ok(false);
        }
        self.save(&next)?;
        *voices = next;
        Ok(true)
    }

    fn save(&self, voices: &[OwnVoice]) -> std::io::Result<()> {
        let file = self.file.as_ref().ok_or_else(|| std::io::Error::other("no config directory"))?;
        crate::resident_claude::write_private(file, &serde_json::to_string_pretty(voices).expect("serializes"))
    }
}

fn load(file: &Path) -> Vec<OwnVoice> {
    let Ok(text) = std::fs::read_to_string(file) else { return Vec::new() };
    serde_json::from_str(&text).unwrap_or_else(|error| {
        warn!(%error, file = %file.display(), "own voices unreadable; none kept");
        Vec::new()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_voice_is_kept_once_per_provider_and_id_and_survives_a_restart() {
        let scratch = crate::voice_keys::tests::scratch("own-voices");
        let own = OwnVoices::new(Some(&scratch));
        let first = own.add("volc", "S_abc", "Me", Some("seed-icl-2.0".to_owned())).unwrap();
        assert!(first.id.starts_with("own-") && first.id.len() == 12);
        let again = own.add("volc", "S_abc", "Me, again", Some("seed-icl-1.0".to_owned())).unwrap();
        assert_eq!(again.id, first.id);
        own.add("elevenlabs", "abc", "Studio", None).unwrap();

        let reloaded = OwnVoices::new(Some(&scratch));
        assert_eq!(reloaded.list().len(), 2);
        assert_eq!(reloaded.get(&first.id).unwrap().name, "Me, again");
        assert_eq!(reloaded.get(&first.id).unwrap().resource.as_deref(), Some("seed-icl-1.0"));
        assert!(reloaded.remove(&first.id).unwrap());
        assert!(!reloaded.remove(&first.id).unwrap());
        assert_eq!(OwnVoices::new(Some(&scratch)).list().len(), 1);
    }

    #[test]
    fn without_a_config_directory_nothing_is_kept() {
        assert!(OwnVoices::new(None).add("elevenlabs", "abc", "Me", None).is_err());
    }
}
