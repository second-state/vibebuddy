//! A firmware zip the user picked (Device → Flash from file…), the counterpart of the Mac's `FirmwarePackage`:
//! the three images are found by name anywhere in the zip, checked for their magic numbers, and unpacked flat into one
//! directory, which is what the daemon's flash endpoint reads.

use std::io::Read;
use std::path::{Path, PathBuf};

use crate::i18n::tr;

const BOOTLOADER: &str = "bootloader.bin";
const PARTITION_TABLE: &str = "partition-table.bin";
const APP: &str = "vibebuddy-fw.bin";
const BUILD: &str = "build.txt";
const VERSION: &str = "version.txt";

#[derive(Clone, Debug, PartialEq)]
pub struct Package {
    pub directory: PathBuf,
    /// "hash date time", as the box reports it; without build.txt, the version string inside the app image.
    pub build: String,
    /// From version.txt; packages from before ADR-0010 have none.
    pub version: Option<String>,
}

impl Package {
    /// As the box's firmware is shown: version first, then the build ID.
    pub fn label(&self) -> String {
        match &self.version {
            Some(version) => format!("{version} · {}", self.build),
            None => self.build.clone(),
        }
    }
}

/// Asks for a zip and unpacks it into a fresh temporary directory. `Ok(None)` when the user cancels.
pub async fn pick() -> Result<Option<Package>, String> {
    #[cfg(target_os = "linux")]
    let file = rfd::AsyncFileDialog::new()
        .set_title(tr("Choose a firmware package", &[]))
        .add_filter("Zip", &["zip"])
        .pick_file()
        .await
        .map(|file| file.path().to_path_buf());
    #[cfg(not(target_os = "linux"))]
    let file: Option<PathBuf> = None;
    let Some(zip) = file else { return Ok(None) };
    let name = zip.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let bytes = tokio::fs::read(&zip).await.map_err(|error| format!("{name}: {error}"))?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |time| time.as_nanos());
    let directory = std::env::temp_dir().join(format!("vibebuddy-firmware-{}-{stamp}", std::process::id()));
    unpack(&bytes, &directory).map(Some).map_err(|error| error.unwrap_or_else(|| tr("Couldn't unzip %@", &[&name])))
}

/// `Err(None)` means the file isn't a readable zip at all.
fn unpack(zip: &[u8], directory: &Path) -> Result<Package, Option<String>> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip)).map_err(|_| None)?;
    std::fs::create_dir_all(directory).map_err(|error| Some(error.to_string()))?;
    let wanted = [BOOTLOADER, PARTITION_TABLE, APP, BUILD, VERSION];
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).map_err(|_| None)?;
        let Some(name) = Path::new(file.name()).file_name().and_then(|name| name.to_str()).map(str::to_owned) else {
            continue;
        };
        // The first file of each name wins, as on the Mac.
        if !file.is_file() || !wanted.contains(&name.as_str()) || directory.join(&name).exists() {
            continue;
        }
        let mut contents = Vec::new();
        file.read_to_end(&mut contents).map_err(|_| None)?;
        std::fs::write(directory.join(&name), contents).map_err(|error| Some(error.to_string()))?;
    }
    inspect(directory).map_err(Some)
}

/// Checks the images as the Mac does: an ESP image starts with 0xE9, a partition table entry with 0xAA 0x50, and the
/// app image carries esp_app_desc's magic 0xABCD5432 (little endian) at 0x20.
fn inspect(directory: &Path) -> Result<Package, String> {
    let read = |name: &str| std::fs::read(directory.join(name)).map_err(|_| tr("The firmware package has no %@", &[&name]));
    let not_an_image = |name: &str| tr("%@ is not an ESP32-S3 image", &[&name]);
    let bootloader = read(BOOTLOADER)?;
    let table = read(PARTITION_TABLE)?;
    let app = read(APP)?;
    if bootloader.first() != Some(&0xE9) {
        return Err(not_an_image(BOOTLOADER));
    }
    if !table.starts_with(&[0xAA, 0x50]) {
        return Err(not_an_image(PARTITION_TABLE));
    }
    if app.len() < 0x50 || app[0x20..0x24] != [0x32, 0x54, 0xCD, 0xAB] {
        return Err(not_an_image(APP));
    }
    let text = |name: &str| {
        std::fs::read_to_string(directory.join(name)).ok().map(|text| text.trim().to_owned()).filter(|text| !text.is_empty())
    };
    let build = text(BUILD).unwrap_or_else(|| {
        let version = &app[0x30..0x50];
        String::from_utf8_lossy(&version[..version.iter().position(|byte| *byte == 0).unwrap_or(version.len())]).into_owned()
    });
    Ok(Package { directory: directory.to_path_buf(), build, version: text(VERSION) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn app_image(version: &str) -> Vec<u8> {
        let mut image = vec![0xE9; 0x50];
        image[0x20..0x24].copy_from_slice(&[0x32, 0x54, 0xCD, 0xAB]);
        image[0x30..0x50].fill(0);
        image[0x30..0x30 + version.len()].copy_from_slice(version.as_bytes());
        image
    }

    fn zip(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, contents) in files {
            zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(contents).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vibebuddy-firmware-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_release_zip_unpacks_flat_with_its_version() {
        let app = app_image("v0.4.0");
        let bytes = zip(&[
            ("VibeBuddy-firmware/bootloader.bin", &[0xE9, 1, 2]),
            ("VibeBuddy-firmware/partition-table.bin", &[0xAA, 0x50, 1]),
            ("VibeBuddy-firmware/vibebuddy-fw.bin", &app),
            ("VibeBuddy-firmware/build.txt", b"a1b2c3d 2026-10-07 12:00\n"),
            ("VibeBuddy-firmware/version.txt", b"0.4.0\n"),
            ("VibeBuddy-firmware/licenses/LICENSE", b"text"),
        ]);
        let dir = temp("release");
        let package = unpack(&bytes, &dir).expect("package");
        assert_eq!(package.label(), "0.4.0 · a1b2c3d 2026-10-07 12:00");
        assert!(dir.join(APP).is_file() && !dir.join("LICENSE").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn without_build_txt_the_build_comes_from_the_image() {
        let app = app_image("v0.2.1-38-ga0bffc7");
        let bytes = zip(&[(BOOTLOADER, &[0xE9]), (PARTITION_TABLE, &[0xAA, 0x50]), (APP, &app)]);
        let dir = temp("old");
        let package = unpack(&bytes, &dir).expect("package");
        assert_eq!((package.build.as_str(), package.version), ("v0.2.1-38-ga0bffc7", None));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_missing_or_wrong_image_is_named() {
        let dir = temp("missing");
        let missing = unpack(&zip(&[(BOOTLOADER, &[0xE9]), (APP, &app_image("x"))]), &dir).unwrap_err();
        assert_eq!(missing.as_deref(), Some("The firmware package has no partition-table.bin"));
        let _ = std::fs::remove_dir_all(&dir);
        let wrong = unpack(&zip(&[(BOOTLOADER, b"MZ"), (PARTITION_TABLE, &[0xAA, 0x50]), (APP, &app_image("x"))]), &dir);
        assert_eq!(wrong.unwrap_err().as_deref(), Some("bootloader.bin is not an ESP32-S3 image"));
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(unpack(b"not a zip", &temp("junk")), Err(None));
    }
}
