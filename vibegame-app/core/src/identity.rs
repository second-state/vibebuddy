//! The VibeGame key, and the paired computers as the app keeps them in its data.

use ed25519_dalek::SigningKey;
use serde_json::{Value, json};

use crate::noise;

/// The VibeGame key: an Ed25519 key made once from the system's random source.
pub struct Identity {
    seed: [u8; 32],
}

impl Identity {
    pub fn from_seed(seed: [u8; 32]) -> Self {
        Self { seed }
    }

    /// The seed as kept in the app's data (`identity`); None if the bytes there aren't one.
    pub fn from_saved(bytes: &[u8]) -> Option<Self> {
        Some(Self::from_seed(bytes.try_into().ok()?))
    }

    pub fn seed(&self) -> &[u8; 32] {
        &self.seed
    }

    /// The Ed25519 public key as unpadded base64url: the `key` in TXT and in `link.noise`.
    pub fn key(&self) -> String {
        noise::encode(SigningKey::from_bytes(&self.seed).verifying_key().as_bytes())
    }

    /// The Ed25519 key as an X25519 secret, for Noise.
    pub fn noise_secret(&self) -> [u8; 32] {
        SigningKey::from_bytes(&self.seed).to_scalar_bytes()
    }

    /// The name in TXT and in `vibegame.hello`. QingOS gives an app no device name, so it is the mDNS instance name.
    pub fn name(&self) -> String {
        format!("VibeGame {}", &self.key()[..8])
    }
}

/// A paired computer: the X25519 static key its handshakes show, and the name it gave at pairing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Computer {
    pub key: [u8; 32],
    pub name: String,
}

/// The paired computers as kept in the app's data (`computers`): `[{"key": "<base64url>", "name": "…"}]`.
pub fn save_computers(computers: &[Computer]) -> String {
    Value::Array(computers.iter().map(|c| json!({"key": noise::encode(&c.key), "name": c.name})).collect()).to_string()
}

/// The paired computers from the app's data. Anything that isn't one is skipped.
pub fn load_computers(bytes: &[u8]) -> Vec<Computer> {
    let Ok(Value::Array(items)) = serde_json::from_slice(bytes) else { return Vec::new() };
    items
        .iter()
        .filter_map(|item| {
            let key = noise::decode_key(item["key"].as_str()?)?;
            Some(Computer { key, name: item["name"].as_str().unwrap_or_default().to_owned() })
        })
        .take(crate::server::MAX_COMPUTERS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_and_name_come_from_the_seed() {
        let identity = Identity::from_seed([7; 32]);
        assert_eq!(identity.key().len(), 43);
        assert_eq!(identity.name(), format!("VibeGame {}", &identity.key()[..8]));
        assert!(Identity::from_saved(&[1, 2, 3]).is_none());
    }

    #[test]
    fn computers_round_trip_and_bad_entries_are_skipped() {
        let computers = vec![Computer { key: [1; 32], name: "dragon's MacBook".into() }, Computer { key: [2; 32], name: String::new() }];
        assert_eq!(load_computers(save_computers(&computers).as_bytes()), computers);
        assert_eq!(load_computers(br#"[{"key":"short"},{"name":"no key"}]"#), Vec::new());
        assert_eq!(load_computers(b"not json"), Vec::new());
    }
}
