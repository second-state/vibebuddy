//! What `install.sh` put next to the app: voice packs built from `voices/`, and the firmware of the release this
//! app belongs to. On the Mac both ride inside the app bundle; here they live in `$XDG_DATA_HOME/vibebuddy`.

use std::path::{Path, PathBuf};

use crate::i18n::{self, tr};

pub fn data_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .map(|dir| dir.join("vibebuddy"))
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
        voice("xiaohe2", tr("Xiaohe 2.0", &[]), tr("Chinese · Mandarin · Doubao", &[]), Language::Chinese),
        voice("jessica", "Jessica".to_owned(), tr("English · US · ElevenLabs", &[]), Language::English),
        voice("chris", "Chris".to_owned(), tr("English · US · ElevenLabs", &[]), Language::English),
    ]
}

/// The voices whose pack was installed, those in the UI's language first.
pub fn voices() -> Vec<Voice> {
    let Some(dir) = data_dir() else { return Vec::new() };
    let preferred = if i18n::is_chinese() { Language::Chinese } else { Language::English };
    let mut voices: Vec<Voice> = catalog().into_iter().filter(|voice| voice_pack(&dir, voice.id).is_file()).collect();
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

pub async fn read_voice_pack(id: &'static str) -> Result<Vec<u8>, String> {
    let path = voice_pack(&data_dir().ok_or("HOME is not set")?, id);
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
    let dir = data_dir()?.join("firmware");
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
