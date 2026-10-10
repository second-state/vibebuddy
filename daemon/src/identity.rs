//! This computer's side of pairing (ADR-0012): an Ed25519 key that tells the box, the local network
//! and the Relay which computer is talking, plus the name the box shows for it.

use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ed25519_dalek::SigningKey;

pub struct Identity {
    key: SigningKey,
    /// What the box lists this computer as.
    pub name: String,
}

impl Identity {
    /// Reads the key from `path`, making and saving one the first time. The file holds the secret seed
    /// as base64url and is readable only by its owner.
    pub fn load_or_create(path: &Path, name: String) -> std::io::Result<Self> {
        let seed = match std::fs::read_to_string(path) {
            Ok(text) => URL_SAFE_NO_PAD
                .decode(text.trim())
                .ok()
                .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
                .ok_or_else(|| std::io::Error::other(format!("{} is not a key", path.display())))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut seed = [0u8; 32];
                getrandom::fill(&mut seed).map_err(|error| std::io::Error::other(error.to_string()))?;
                write_private(path, &URL_SAFE_NO_PAD.encode(seed))?;
                seed
            }
            Err(error) => return Err(error),
        };
        Ok(Self { key: SigningKey::from_bytes(&seed), name })
    }

    /// The public key as the box and the Relay see it: 43 characters of base64url.
    pub fn public_key(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.key.verifying_key().to_bytes())
    }
}

fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?;
    file.write_all(text.as_bytes())
}

/// Where the key lives: next to config.json.
pub fn identity_file() -> Option<PathBuf> {
    Some(crate::config::config_dir()?.join("identity"))
}

/// The name people know this computer by: the one in macOS Sharing settings, else the host name.
pub fn computer_name() -> String {
    if cfg!(target_os = "macos")
        && let Ok(output) = std::process::Command::new("/usr/sbin/scutil").args(["--get", "ComputerName"]).output()
        && output.status.success()
        && let Ok(name) = String::from_utf8(output.stdout)
        && !name.trim().is_empty()
    {
        return name.trim().to_owned();
    }
    let mut buffer = [0u8; 256];
    // SAFETY: the buffer is valid for its whole length, and gethostname writes at most that much.
    let result = unsafe { libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len()) };
    let end = buffer.iter().position(|&byte| byte == 0).unwrap_or(buffer.len());
    match std::str::from_utf8(&buffer[..end]) {
        Ok(name) if result == 0 && !name.is_empty() => name.trim_end_matches(".local").to_owned(),
        _ => "computer".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_is_made_once_and_kept() {
        let dir = std::env::temp_dir().join(format!("vibebuddy-identity-{}", std::process::id()));
        let path = dir.join("identity");
        let first = Identity::load_or_create(&path, "a".to_owned()).unwrap();
        let again = Identity::load_or_create(&path, "a".to_owned()).unwrap();
        assert_eq!(first.public_key(), again.public_key());
        assert_eq!(first.public_key().len(), 43);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_damaged_key_file_is_an_error_not_a_new_key() {
        let dir = std::env::temp_dir().join(format!("vibebuddy-identity-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("identity");
        std::fs::write(&path, "nonsense").unwrap();
        assert!(Identity::load_or_create(&path, "a".to_owned()).is_err());
        std::fs::remove_dir_all(dir).ok();
    }
}
