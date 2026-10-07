//! What the Character tab remembers, and the user's drawings: the form of address picked per language,
//! and the user's own Character as last written (its look, kept so the card can show it after a
//! restart, and the Character lending it voice and lines). Kept in the app's config directory, like
//! the language override.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::assets::Language;
use crate::character::Image;

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Settings {
    /// Form of address id per language code ("zh", "en"); a language not here says none.
    #[serde(default)]
    pub address: HashMap<String, String>,
    /// The Character lending voice and lines to the user's own, as last written.
    #[serde(default)]
    pub custom_lender: Option<String>,
    /// The Character lending voice and lines to the robot; none means its five built-in lines.
    #[serde(default)]
    pub robot_lender: Option<String>,
}

fn settings_file() -> Option<PathBuf> {
    crate::config_dir().map(|dir| dir.join("characters.json"))
}

pub fn custom_look_file() -> Option<PathBuf> {
    crate::config_dir().map(|dir| dir.join("custom-look.bin"))
}

impl Settings {
    pub fn load() -> Settings {
        settings_file()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(path) = settings_file() else { return };
        write(&path, serde_json::to_string_pretty(self).unwrap_or_default().as_bytes());
    }

    pub fn address(&self, language: Language) -> Option<&str> {
        self.address.get(language.code()).map(String::as_str)
    }

    pub fn set_address(&mut self, language: Language, form: Option<&str>) {
        match form {
            Some(form) => self.address.insert(language.code().to_owned(), form.to_owned()),
            None => self.address.remove(language.code()),
        };
    }
}

pub fn write(path: &Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, bytes);
}

/// Asks for one to four drawings through the desktop's file chooser and decodes them, in file name
/// order (normal, eyes closed, happy, sad). Ok(None) when the user cancels.
pub async fn choose_drawings() -> Result<Option<Vec<Image>>, String> {
    #[cfg(target_os = "linux")]
    let files = rfd::AsyncFileDialog::new().add_filter("Images", &["png", "jpg", "jpeg"]).pick_files().await;
    #[cfg(not(target_os = "linux"))]
    let files: Option<Vec<PathBuf>> = None;
    let Some(files) = files else { return Ok(None) };
    #[cfg(target_os = "linux")]
    let mut paths: Vec<PathBuf> = files.iter().map(|file| file.path().to_path_buf()).collect();
    #[cfg(not(target_os = "linux"))]
    let mut paths = files;
    paths.sort();
    if !(1..=4).contains(&paths.len()) {
        return Err(crate::i18n::tr("Pick one to four PNG or JPEG images.", &[]));
    }
    tokio::task::spawn_blocking(move || paths.iter().map(|path| decode(path)).collect::<Result<Vec<_>, _>>().map(Some))
        .await
        .map_err(|error| error.to_string())?
}

fn decode(path: &Path) -> Result<Image, String> {
    let image = image::open(path).map_err(|error| format!("{}: {error}", path.display()))?.to_rgba8();
    Ok(Image { width: image.width() as usize, height: image.height() as usize, pixels: image.into_raw() })
}
