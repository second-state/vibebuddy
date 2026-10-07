//! The update manifest (ADR-0010): what can be installed, signed so that whoever controls the host can't make a box
//! flash their firmware. The daemon verifies it with the public key; CI builds and signs it (`build`).

use std::collections::BTreeMap;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ed25519_dalek::{Signature, Signer, Verifier};
pub use ed25519_dalek::{SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};

pub mod appcast;
pub mod build;

/// Bumped only when an old daemon would misread the manifest; adding fields doesn't need it.
pub const SCHEMA: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema: u32,
    /// When CI built it; a daemon refuses one older than the last it accepted, so an old copy can't hide an update.
    pub generated_at: String,
    /// Apps older than this show a banner that can't be dismissed.
    pub min_supported_app: String,
    /// The latest App per platform, keyed `macos-arm64`, `linux-x86_64`.
    pub app: BTreeMap<String, Download>,
    /// Every released firmware, newest first, so an App too old for the newest one can still find one it can run.
    pub firmware: Vec<Firmware>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Download {
    pub version: String,
    pub url: String,
    pub sha256: String,
    pub notes: Notes,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Firmware {
    /// The oldest App that can run this firmware.
    pub min_app: String,
    #[serde(flatten)]
    pub download: Download,
}

/// Release notes in Markdown by language (`en`, `zh-Hans`); `en` is always there and is the fallback.
pub type Notes = BTreeMap<String, String>;

/// What is served: the manifest as exact JSON text plus an Ed25519 signature over those bytes, so verifying never
/// depends on how JSON gets re-serialized.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signed {
    pub manifest: String,
    pub signature: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    Key(String),
    Encoding(String),
    BadSignature,
    Json(String),
    UnknownSchema(u32),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Key(message) => write!(f, "bad key: {message}"),
            Error::Encoding(message) => write!(f, "bad base64: {message}"),
            Error::BadSignature => f.write_str("the signature doesn't match"),
            Error::Json(message) => write!(f, "bad manifest: {message}"),
            Error::UnknownSchema(schema) => write!(f, "unknown manifest schema {schema}"),
        }
    }
}

impl std::error::Error for Error {}

/// Keys are 32 raw bytes in base64: the private key is the Ed25519 seed.
pub fn signing_key(base64: &str) -> Result<SigningKey, Error> {
    Ok(SigningKey::from_bytes(&key_bytes(base64)?))
}

pub fn verifying_key(base64: &str) -> Result<VerifyingKey, Error> {
    VerifyingKey::from_bytes(&key_bytes(base64)?).map_err(|error| Error::Key(error.to_string()))
}

/// A fresh private key and its public key, both base64.
pub fn generate_key() -> Result<(String, String), Error> {
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).map_err(|error| Error::Key(error.to_string()))?;
    let key = SigningKey::from_bytes(&seed);
    Ok((STANDARD.encode(seed), STANDARD.encode(key.verifying_key().as_bytes())))
}

pub fn sign(manifest: &Manifest, key: &SigningKey) -> Signed {
    let text = serde_json::to_string_pretty(manifest).expect("a manifest always serializes");
    let signature = key.sign(text.as_bytes());
    Signed { manifest: text, signature: STANDARD.encode(signature.to_bytes()) }
}

/// Checks the signature before reading a single field of the manifest.
pub fn verify(signed: &Signed, key: &VerifyingKey) -> Result<Manifest, Error> {
    let bytes = STANDARD.decode(signed.signature.trim()).map_err(|error| Error::Encoding(error.to_string()))?;
    let signature = Signature::from_slice(&bytes).map_err(|_| Error::BadSignature)?;
    key.verify(signed.manifest.as_bytes(), &signature).map_err(|_| Error::BadSignature)?;
    let manifest: Manifest = serde_json::from_str(&signed.manifest).map_err(|error| Error::Json(error.to_string()))?;
    if manifest.schema != SCHEMA {
        return Err(Error::UnknownSchema(manifest.schema));
    }
    Ok(manifest)
}

fn key_bytes(base64: &str) -> Result<[u8; 32], Error> {
    let bytes = STANDARD.decode(base64.trim()).map_err(|error| Error::Encoding(error.to_string()))?;
    bytes.try_into().map_err(|bytes: Vec<u8>| Error::Key(format!("{} bytes, expected 32", bytes.len())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> Manifest {
        let notes = Notes::from([("en".to_owned(), "- Fixes".to_owned())]);
        Manifest {
            schema: SCHEMA,
            generated_at: "2026-10-07T12:00:00Z".to_owned(),
            min_supported_app: "0.3.2".to_owned(),
            app: BTreeMap::from([(
                "macos-arm64".to_owned(),
                Download { version: "0.4.0".to_owned(), url: "https://example/app.dmg".to_owned(), sha256: "aa".to_owned(), notes: notes.clone() },
            )]),
            firmware: vec![Firmware {
                min_app: "0.4.0".to_owned(),
                download: Download { version: "0.3.3".to_owned(), url: "https://example/fw.zip".to_owned(), sha256: "bb".to_owned(), notes },
            }],
        }
    }

    fn keys() -> (SigningKey, VerifyingKey) {
        let (private, public) = generate_key().unwrap();
        (signing_key(&private).unwrap(), verifying_key(&public).unwrap())
    }

    #[test]
    fn a_signed_manifest_verifies_and_reads_back() {
        let (private, public) = keys();
        let signed = sign(&manifest(), &private);
        assert_eq!(verify(&signed, &public), Ok(manifest()));
    }

    #[test]
    fn a_tampered_manifest_is_rejected() {
        let (private, public) = keys();
        let mut signed = sign(&manifest(), &private);
        signed.manifest = signed.manifest.replace("https://example/fw.zip", "https://evil/fw.zip");
        assert_eq!(verify(&signed, &public), Err(Error::BadSignature));
    }

    #[test]
    fn another_key_is_rejected() {
        let (private, _) = keys();
        let (_, other) = keys();
        assert_eq!(verify(&sign(&manifest(), &private), &other), Err(Error::BadSignature));
    }

    #[test]
    fn a_garbled_signature_is_rejected() {
        let (private, public) = keys();
        let mut signed = sign(&manifest(), &private);
        signed.signature = STANDARD.encode([0u8; 10]);
        assert_eq!(verify(&signed, &public), Err(Error::BadSignature));
    }

    #[test]
    fn an_unknown_schema_is_refused_even_when_signed() {
        let (private, public) = keys();
        let mut future = manifest();
        future.schema = SCHEMA + 1;
        assert_eq!(verify(&sign(&future, &private), &public), Err(Error::UnknownSchema(SCHEMA + 1)));
    }

    #[test]
    fn firmware_fields_sit_flat_in_the_json() {
        let json = serde_json::to_value(&manifest().firmware[0]).unwrap();
        assert_eq!(json["version"], "0.3.3");
        assert_eq!(json["min_app"], "0.4.0");
    }
}
