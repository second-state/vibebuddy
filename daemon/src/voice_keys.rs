//! The user's own keys for spoken replies (ADR-0020, "Own key"): an ElevenLabs key and a Volcano Engine key, in
//! `voice-keys.json` in the config directory, `{"elevenlabs"?, "volc"?}`, mode 0600. A file of its own, as
//! `speech.json` is, because `PUT /v1/config` replaces the whole config and an older App would wipe a key it doesn't
//! know. A reply whose voice is from a provider with an own key goes straight to that provider ([`crate::synthesis`]).
//! Only which keys are set, and whether a provider refused one, leave the daemon; a key is never sent back or logged.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use tracing::warn;

pub const FILE: &str = "voice-keys.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    ElevenLabs,
    Volc,
}

impl Provider {
    /// Its name in the file, the local API and a voice's `<vendor>:` prefix.
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "elevenlabs" => Some(Self::ElevenLabs),
            "volc" => Some(Self::Volc),
            _ => None,
        }
    }

    /// The provider of a voice service voice (`elevenlabs:<id>`, `volc:<speaker>`), and its id there.
    pub fn of(voice: &str) -> Option<(Self, &str)> {
        let (vendor, id) = voice.split_once(':')?;
        Some((Self::named(vendor)?, id))
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct Saved {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    elevenlabs: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    volc: Option<String>,
}

impl Saved {
    fn key(&mut self, provider: Provider) -> &mut Option<String> {
        match provider {
            Provider::ElevenLabs => &mut self.elevenlabs,
            Provider::Volc => &mut self.volc,
        }
    }
}

/// In `/v1/status`: which keys are set, never a key.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct VoiceKeysStatus {
    pub elevenlabs: KeyStatus,
    pub volc: KeyStatus,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct KeyStatus {
    pub set: bool,
    /// The provider last answered 401 or 403 to it; cleared when it takes it again or the key changes.
    pub refused: bool,
}

pub struct VoiceKeys {
    /// None without a config directory, and in tests unless one sets it; then no key can be set.
    file: Option<PathBuf>,
    saved: Mutex<Saved>,
    refused: Mutex<[bool; 2]>,
    changed: watch::Sender<()>,
}

impl VoiceKeys {
    pub fn new(config_dir: Option<&Path>) -> Self {
        let file = config_dir.map(|dir| dir.join(FILE));
        let saved = file.as_deref().map(load).unwrap_or_default();
        Self { file, saved: Mutex::new(saved), refused: Mutex::default(), changed: watch::channel(()).0 }
    }

    /// Changes each time a key is set or cleared, or a provider refuses or takes one.
    pub fn watch(&self) -> watch::Receiver<()> {
        self.changed.subscribe()
    }

    pub fn status(&self) -> VoiceKeysStatus {
        let mut saved = self.saved.lock().unwrap().clone();
        let refused = *self.refused.lock().unwrap();
        let mut of = |provider: Provider| KeyStatus { set: saved.key(provider).is_some(), refused: refused[provider as usize] };
        VoiceKeysStatus { elevenlabs: of(Provider::ElevenLabs), volc: of(Provider::Volc) }
    }

    pub fn key(&self, provider: Provider) -> Option<String> {
        self.saved.lock().unwrap().key(provider).clone()
    }

    /// Sets each key, or clears it with None or an empty one, and saves them all at once.
    pub fn set(&self, changes: &[(Provider, Option<String>)]) -> std::io::Result<()> {
        let Some(file) = &self.file else {
            return Err(std::io::Error::other("no config directory"));
        };
        let mut saved = self.saved.lock().unwrap();
        let mut next = saved.clone();
        for (provider, key) in changes {
            *next.key(*provider) = key.as_deref().map(str::trim).filter(|key| !key.is_empty()).map(str::to_owned);
        }
        crate::resident_claude::write_private(file, &serde_json::to_string(&next).expect("serializes"))?;
        *saved = next;
        let mut refused = self.refused.lock().unwrap();
        for (provider, _) in changes {
            refused[*provider as usize] = false;
        }
        self.changed.send_replace(());
        Ok(())
    }

    /// What the provider made of the key on its last call.
    pub fn refused(&self, provider: Provider, refused: bool) {
        let mut flags = self.refused.lock().unwrap();
        if flags[provider as usize] != refused {
            flags[provider as usize] = refused;
            self.changed.send_replace(());
        }
    }
}

fn load(path: &Path) -> Saved {
    let Ok(text) = std::fs::read_to_string(path) else { return Saved::default() };
    serde_json::from_str(&text).unwrap_or_else(|_| {
        warn!(path = %path.display(), "the own keys file is not a JSON object of keys; ignored");
        Saved::default()
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    pub(crate) fn scratch(tag: &str) -> PathBuf {
        let mut bytes = [0u8; 8];
        getrandom::fill(&mut bytes).unwrap();
        std::env::temp_dir().join(format!("vibebuddy-voice-keys-{tag}-{}-{}", std::process::id(), u64::from_le_bytes(bytes)))
    }

    #[test]
    fn keys_are_kept_0600_and_the_status_says_only_which_are_set() {
        let dir = scratch("file");
        let keys = VoiceKeys::new(Some(&dir));
        assert_eq!(keys.status(), VoiceKeysStatus::default());
        keys.set(&[(Provider::ElevenLabs, Some(" sk_eleven_secret \n".to_owned())), (Provider::Volc, None)]).unwrap();
        let path = dir.join(FILE);
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), r#"{"elevenlabs":"sk_eleven_secret"}"#);
        let status = keys.status();
        assert!(status.elevenlabs.set && !status.volc.set);
        assert!(!serde_json::to_string(&status).unwrap().contains("secret"));

        // Read back at start; an empty key clears it, and a key left out stays.
        let again = VoiceKeys::new(Some(&dir));
        assert_eq!(again.key(Provider::ElevenLabs).as_deref(), Some("sk_eleven_secret"));
        again.set(&[(Provider::Volc, Some("volc_secret".to_owned()))]).unwrap();
        again.set(&[(Provider::ElevenLabs, Some(" ".to_owned()))]).unwrap();
        assert_eq!((again.key(Provider::ElevenLabs), again.key(Provider::Volc).as_deref()), (None, Some("volc_secret")));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_refusal_shows_until_the_key_changes() {
        let dir = scratch("refused");
        let keys = VoiceKeys::new(Some(&dir));
        keys.set(&[(Provider::Volc, Some("k".to_owned()))]).unwrap();
        let changes = keys.watch();
        keys.refused(Provider::Volc, true);
        assert!(changes.has_changed().unwrap());
        assert_eq!(keys.status().volc, KeyStatus { set: true, refused: true });
        assert!(!keys.status().elevenlabs.refused);
        keys.set(&[(Provider::Volc, Some("k2".to_owned()))]).unwrap();
        assert_eq!(keys.status().volc, KeyStatus { set: true, refused: false });
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn voices_name_their_provider() {
        assert_eq!(Provider::of("elevenlabs:abc"), Some((Provider::ElevenLabs, "abc")));
        assert_eq!(Provider::of("volc:zh_female_wanwanxiaohe_moon_bigtts"), Some((Provider::Volc, "zh_female_wanwanxiaohe_moon_bigtts")));
        assert_eq!(Provider::of("en-US-JennyNeural"), None);
    }
}
