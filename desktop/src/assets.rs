//! What was installed next to the app: the Character packs from `characters/`, and the firmware of the release this app
//! belongs to. On the Mac both ride inside the app bundle. Here `install.sh` puts them in `~/.local/share/vibebuddy`
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    Chinese,
    English,
}

/// The Mac app's `VoiceCatalogEntry.all`, with the same English keys for names and tags.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Voice {
    pub id: &'static str,
    pub name: String,
    pub tag: String,
    pub language: Language,
}

fn catalog() -> Vec<Voice> {
    let voice = |id, name: String, tag: String, language| Voice { id, name, tag, language };
    vec![
        voice(
            "wanwanxiaohe",
            tr("Wanwan Xiaohe", &[]),
            tr("Chinese · Taiwanese accent · Doubao · same voice as Xiaozhi", &[]),
            Language::Chinese,
        ),
        voice("ahu", tr("Ahu", &[]), tr("Chinese · Mandarin · Doubao", &[]), Language::Chinese),
        voice("amanda", "Amanda".to_owned(), tr("English · Doubao", &[]), Language::English),
        voice("jackson", "Jackson".to_owned(), tr("English · Doubao", &[]), Language::English),
    ]
}

/// The voices whose pack was installed, those in the UI's language first.
pub fn voices() -> Vec<Voice> {
    let preferred = if i18n::is_chinese() { Language::Chinese } else { Language::English };
    let mut voices: Vec<Voice> = catalog().into_iter().filter(|voice| find_voice_pack(voice.id).is_some()).collect();
    voices.sort_by_key(|voice| voice.language != preferred);
    voices
}

pub fn voice_name(id: &str) -> String {
    if id == "builtin" {
        return tr("Built-in voice (Jessica)", &[]);
    }
    catalog().into_iter().find(|voice| voice.id == id).map(|voice| voice.name).unwrap_or_else(|| id.to_owned())
}

fn voice_pack(dir: &Path, id: &str) -> PathBuf {
    dir.join("voices").join(format!("{id}.bin"))
}

fn find_voice_pack(id: &str) -> Option<PathBuf> {
    data_dirs().into_iter().map(|dir| voice_pack(&dir, id)).find(|path| path.is_file())
}

pub async fn read_voice_pack(id: &'static str) -> Result<Vec<u8>, String> {
    let path = find_voice_pack(id).ok_or_else(|| format!("no voice pack named {id} is installed"))?;
    tokio::fs::read(&path).await.map_err(|error| format!("{}: {error}", path.display()))
}

/// The three images the daemon flashes, and the build they carry ("hash date time", like the box reports).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Firmware {
    pub bootloader: PathBuf,
    pub partition_table: PathBuf,
    pub app: PathBuf,
    pub build: String,
}

pub fn firmware() -> Option<Firmware> {
    data_dirs().iter().find_map(|dir| firmware_in(&dir.join("firmware")))
}

fn firmware_in(dir: &Path) -> Option<Firmware> {
    let build = std::fs::read_to_string(dir.join("build.txt")).ok()?.trim().to_owned();
    let firmware = Firmware {
        bootloader: dir.join("bootloader.bin"),
        partition_table: dir.join("partition-table.bin"),
        app: dir.join("vibebuddy-fw.bin"),
        build,
    };
    [&firmware.bootloader, &firmware.partition_table, &firmware.app]
        .iter()
        .all(|path| path.is_file())
        .then_some(firmware)
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
    fn relative_entries_are_ignored() {
        let dirs = search_dirs(Some("relative".into()), None, Some("share:/opt/share".into()));
        assert_eq!(dirs, [PathBuf::from("/opt/share/vibebuddy")]);
    }
}
