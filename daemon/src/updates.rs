//! Update checks (ADR-0010): read the signed update manifest, work out what to offer, and keep the offered firmware
//! on disk so flashing and recovering a box work offline afterwards. Nothing is ever installed from here; the App
//! and the desktop show what this finds and the user decides.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Local};
use semver::Version;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tracing::warn;
use vibebuddy_manifest::{Firmware, Manifest, Notes, Signed, VerifyingKey};

use crate::status::DeviceState;

/// Where the manifest lives: R2 behind our own domain, so it can move or gain a Worker without an App release.
/// `VIBEBUDDY_MANIFEST_URL` points somewhere else for testing.
const MANIFEST_URL: Option<&str> = Some("https://updates.korekore.ai/vibebuddy/manifest.json");

/// The key CI signs the manifest with (`tools/setup-update-signing.sh`).
const PUBLIC_KEY: &str = include_str!("../../manifest/manifest-key.pub");

pub const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// The files a firmware zip must hold to be flashed; build.txt and version.txt only describe it.
const IMAGES: [&str; 3] = ["bootloader.bin", "partition-table.bin", "vibebuddy-fw.bin"];
const DESCRIPTIONS: [&str; 2] = ["build.txt", "version.txt"];
/// A firmware zip is about 2 MB; anything far bigger isn't one.
const MAX_DOWNLOAD: u64 = 32 * 1024 * 1024;
const MAX_MANIFEST: u64 = 1024 * 1024;

/// Checking is on by default only in builds CI makes (`VIBEBUDDY_RELEASE_BUILD` at compile time); someone building
/// from source hasn't agreed to anything contacting us.
pub fn enabled_by_default() -> bool {
    option_env!("VIBEBUDDY_RELEASE_BUILD").is_some()
}

/// The manifest's platform key for this build.
pub fn platform() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("macos-arm64"),
        ("linux", "x86_64") => Some("linux-x86_64"),
        _ => None,
    }
}

/// Where the manifest comes from and the key it must be signed with.
#[derive(Clone)]
pub struct Source {
    pub url: String,
    pub key: VerifyingKey,
}

impl Source {
    pub fn configured() -> Option<Self> {
        let url = std::env::var("VIBEBUDDY_MANIFEST_URL").ok().filter(|url| !url.is_empty()).or(MANIFEST_URL.map(str::to_owned))?;
        let key = match vibebuddy_manifest::verifying_key(PUBLIC_KEY) {
            Ok(key) => key,
            Err(error) => {
                warn!(%error, "the compiled-in manifest key is unusable; not checking for updates");
                return None;
            }
        };
        Some(Self { url, key })
    }
}

/// What the daemon knows about updates; one per daemon, behind a lock in the app state.
pub struct Updates {
    pub source: Option<Source>,
    /// `<state dir>/updates`: the last accepted manifest and the offered firmware. None keeps everything in memory.
    dir: Option<PathBuf>,
    app_version: Version,
    manifest: Option<Manifest>,
    last_check: Option<DateTime<Local>>,
    error: Option<String>,
}

/// What `/v1/status` says about updates.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct UpdateStatus {
    /// Whether the daemon checks: switched on, and there is somewhere to check.
    pub enabled: bool,
    pub last_check: Option<DateTime<Local>>,
    pub error: Option<String>,
    /// A newer App for this platform, if any.
    pub app: Option<AppOffer>,
    /// The running App is older than the manifest's `min_supported_app`.
    pub unsupported_app: bool,
    /// The firmware this App would flash: the newest one it can run.
    pub firmware: Option<FirmwareOffer>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AppOffer {
    pub version: String,
    pub url: String,
    pub notes: Notes,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FirmwareOffer {
    pub version: String,
    pub notes: Notes,
    /// The unpacked images, once downloaded and checked; the three files `/v1/device/firmware` takes.
    pub directory: Option<PathBuf>,
    /// Worth offering to the connected box: it runs an older, unversioned or dirty build. False with no box.
    pub newer_than_box: bool,
}

impl Updates {
    /// Picks up the manifest accepted last time, checked again, so what was offered before is offered offline.
    pub fn new(source: Option<Source>, dir: Option<PathBuf>) -> Self {
        let app_version = Version::parse(env!("CARGO_PKG_VERSION")).expect("the crate version is semver");
        let mut updates = Self { source, dir, app_version, manifest: None, last_check: None, error: None };
        if let (Some(source), Some(path)) = (&updates.source, updates.manifest_path()) {
            updates.manifest = std::fs::read_to_string(path).ok().and_then(|text| verify(&text, &source.key).ok());
        }
        updates
    }

    pub fn status(&self, device: &DeviceState, enabled: bool) -> UpdateStatus {
        let manifest = self.manifest.as_ref();
        let app = manifest
            .and_then(|manifest| manifest.app.get(platform()?))
            .filter(|app| parse(&app.version).is_some_and(|version| version > self.app_version))
            .map(|app| AppOffer { version: app.version.clone(), url: app.url.clone(), notes: app.notes.clone() });
        let unsupported_app = manifest
            .and_then(|manifest| parse(&manifest.min_supported_app))
            .is_some_and(|minimum| self.app_version < minimum);
        let firmware = manifest.and_then(|manifest| pick_firmware(manifest, &self.app_version)).map(|firmware| FirmwareOffer {
            version: firmware.download.version.clone(),
            notes: firmware.download.notes.clone(),
            directory: self.firmware_dir(&firmware.download.version).filter(|dir| unpacked(dir)),
            newer_than_box: device.connected && offer_to_box(&firmware.download.version, device),
        });
        UpdateStatus {
            enabled: enabled && self.source.is_some(),
            last_check: self.last_check,
            error: self.error.clone(),
            app,
            unsupported_app,
            firmware,
        }
    }

    /// Takes a freshly fetched manifest, unless it is older than the one already held: an old copy served again
    /// must not hide an update. Returns the firmware that should be on disk and isn't yet.
    pub fn accept(&mut self, signed_text: &str, manifest: Manifest) -> Result<Option<Firmware>, String> {
        if let Some(current) = &self.manifest
            && older(&manifest.generated_at, &current.generated_at)
        {
            return Err(format!("the manifest from {} is older than the one from {}", manifest.generated_at, current.generated_at));
        }
        if let Some(path) = self.manifest_path() {
            write_file(&path, signed_text.as_bytes()).map_err(|error| format!("{}: {error}", path.display()))?;
        }
        let wanted = pick_firmware(&manifest, &self.app_version).cloned();
        self.manifest = Some(manifest);
        Ok(wanted.filter(|firmware| self.firmware_dir(&firmware.download.version).is_some_and(|dir| !unpacked(&dir))))
    }

    pub fn finish(&mut self, error: Option<String>) {
        self.last_check = Some(Local::now());
        self.error = error;
    }

    pub fn firmware_root(&self) -> Option<PathBuf> {
        Some(self.dir.as_ref()?.join("firmware"))
    }

    fn firmware_dir(&self, version: &str) -> Option<PathBuf> {
        Some(self.firmware_root()?.join(version))
    }

    fn manifest_path(&self) -> Option<PathBuf> {
        Some(self.dir.as_ref()?.join("manifest.json"))
    }
}

/// The newest firmware the running App can run. The manifest lists every release, so an App too old for the
/// latest still finds one.
pub fn pick_firmware<'a>(manifest: &'a Manifest, app: &Version) -> Option<&'a Firmware> {
    manifest
        .firmware
        .iter()
        .filter(|firmware| parse(&firmware.min_app).is_some_and(|minimum| minimum <= *app))
        .filter_map(|firmware| Some((parse(&firmware.download.version)?, firmware)))
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, firmware)| firmware)
}

/// Offer a firmware to the box when it runs something older, something without a version (firmware from before
/// versions, or another firmware entirely), or a dirty build of the same version or older. Never a downgrade.
pub fn offer_to_box(offered: &str, device: &DeviceState) -> bool {
    // Released firmware is built for the box; on other hardware it starts nothing.
    if device.unsupported_board.is_some() {
        return false;
    }
    let Some(offered) = parse(offered) else { return false };
    let Some(running) = device.firmware_version.as_deref().and_then(parse) else { return true };
    let dirty = device.firmware_build.as_deref().is_some_and(|build| build.contains("-dirty"));
    offered > running || (dirty && offered == running)
}

/// Fetches the manifest and checks its signature; returns the exact text (to keep on disk) and what it says.
/// Only the platform and the two versions are sent.
pub fn fetch(source: &Source, app_version: &str, firmware_version: Option<&str>) -> Result<(String, Manifest), String> {
    let mut request = agent().get(&source.url).query("app", app_version);
    if let Some(platform) = platform() {
        request = request.query("platform", platform);
    }
    if let Some(firmware) = firmware_version {
        request = request.query("firmware", firmware);
    }
    let mut response = request.call().map_err(|error| format!("fetching the manifest: {error}"))?;
    let text = response
        .body_mut()
        .with_config()
        .limit(MAX_MANIFEST)
        .read_to_string()
        .map_err(|error| format!("reading the manifest: {error}"))?;
    let manifest = verify(&text, &source.key)?;
    Ok((text, manifest))
}

/// Downloads a firmware zip, checks its sha256 and unpacks the images into `root/<version>`; a half-done download
/// leaves nothing behind that passes for unpacked. Older versions in `root` are removed.
pub fn download(firmware: &Firmware, root: &Path) -> Result<PathBuf, String> {
    let url = &firmware.download.url;
    let mut response = agent().get(url).call().map_err(|error| format!("downloading {url}: {error}"))?;
    let mut bytes = Vec::new();
    response
        .body_mut()
        .with_config()
        .limit(MAX_DOWNLOAD)
        .reader()
        .read_to_end(&mut bytes)
        .map_err(|error| format!("downloading {url}: {error}"))?;
    let digest = hex(&Sha256::digest(&bytes));
    if !digest.eq_ignore_ascii_case(&firmware.download.sha256) {
        return Err(format!("{url} has sha256 {digest}, the manifest says {}", firmware.download.sha256));
    }
    let version = &firmware.download.version;
    let staging = root.join(format!(".{version}.partial"));
    let _ = std::fs::remove_dir_all(&staging);
    unpack(&bytes, &staging)?;
    let target = root.join(version);
    let _ = std::fs::remove_dir_all(&target);
    std::fs::rename(&staging, &target).map_err(|error| format!("{}: {error}", target.display()))?;
    // Only the offered firmware is kept; an App never goes back to an older one.
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            if entry.file_name() != std::ffi::OsStr::new(version) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
    Ok(target)
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(120)))
        .tls_config(ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build())
        .user_agent(concat!("vibebuddyd/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

fn verify(text: &str, key: &VerifyingKey) -> Result<Manifest, String> {
    let signed: Signed = serde_json::from_str(text).map_err(|error| format!("not a signed manifest: {error}"))?;
    vibebuddy_manifest::verify(&signed, key).map_err(|error| error.to_string())
}

/// The images can sit anywhere in the zip (the App's firmware folder also carries licenses); they're found by name.
fn unpack(zip: &[u8], dir: &Path) -> Result<(), String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip)).map_err(|error| format!("not a zip: {error}"))?;
    std::fs::create_dir_all(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).map_err(|error| error.to_string())?;
        let Some(name) = Path::new(file.name()).file_name().and_then(|name| name.to_str()).map(str::to_owned) else { continue };
        if !file.is_file() || !(IMAGES.contains(&name.as_str()) || DESCRIPTIONS.contains(&name.as_str())) || dir.join(&name).exists() {
            continue;
        }
        let mut contents = Vec::new();
        file.read_to_end(&mut contents).map_err(|error| format!("{name}: {error}"))?;
        std::fs::write(dir.join(&name), contents).map_err(|error| format!("{name}: {error}"))?;
    }
    if let Some(missing) = IMAGES.iter().find(|name| !dir.join(name).is_file()) {
        let _ = std::fs::remove_dir_all(dir);
        return Err(format!("the firmware zip has no {missing}"));
    }
    Ok(())
}

fn unpacked(dir: &Path) -> bool {
    IMAGES.iter().all(|name| dir.join(name).is_file())
}

fn write_file(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let partial = path.with_extension("partial");
    std::fs::write(&partial, contents)?;
    std::fs::rename(partial, path)
}

fn older(candidate: &str, current: &str) -> bool {
    match (DateTime::parse_from_rfc3339(candidate), DateTime::parse_from_rfc3339(current)) {
        (Ok(candidate), Ok(current)) => candidate < current,
        // An unreadable date can't be ordered; refusing it keeps what is already trusted.
        _ => true,
    }
}

fn parse(version: &str) -> Option<Version> {
    Version::parse(version).ok()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::io::Write;

    use axum::Router;
    use axum::routing::get;
    use vibebuddy_manifest::{Download, SCHEMA, SigningKey};

    use super::*;

    fn notes(text: &str) -> Notes {
        Notes::from([("en".to_owned(), text.to_owned())])
    }

    fn firmware(version: &str, min_app: &str, url: &str, sha256: &str) -> Firmware {
        Firmware {
            min_app: min_app.to_owned(),
            download: Download { version: version.to_owned(), url: url.to_owned(), sha256: sha256.to_owned(), notes: notes(version) },
        }
    }

    fn manifest(generated_at: &str, firmware: Vec<Firmware>) -> Manifest {
        let app = platform()
            .map(|platform| {
                let app = Download { version: "99.0.0".to_owned(), url: "https://example/app".to_owned(), sha256: "00".to_owned(), notes: notes("app") };
                BTreeMap::from([(platform.to_owned(), app)])
            })
            .unwrap_or_default();
        Manifest { schema: SCHEMA, generated_at: generated_at.to_owned(), min_supported_app: "0.1.0".to_owned(), app, firmware }
    }

    fn keys() -> (SigningKey, VerifyingKey) {
        let (private, public) = vibebuddy_manifest::generate_key().unwrap();
        (vibebuddy_manifest::signing_key(&private).unwrap(), vibebuddy_manifest::verifying_key(&public).unwrap())
    }

    fn signed_text(manifest: &Manifest, key: &SigningKey) -> String {
        serde_json::to_string(&vibebuddy_manifest::sign(manifest, key)).unwrap()
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vibebuddy-updates-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn box_running(version: Option<&str>, build: &str) -> DeviceState {
        DeviceState {
            connected: true,
            firmware_version: version.map(str::to_owned),
            firmware_build: Some(build.to_owned()),
            ..DeviceState::default()
        }
    }

    /// A firmware zip the way CI cuts it: the images inside a folder, next to things that aren't images.
    fn firmware_zip(with_app_image: bool) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        let mut files = vec!["firmware/bootloader.bin", "firmware/partition-table.bin", "firmware/version.txt", "firmware/licenses/LICENSE"];
        if with_app_image {
            files.push("firmware/vibebuddy-fw.bin");
        }
        for name in files {
            zip.start_file(name, options).unwrap();
            zip.write_all(name.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    /// Serves `/manifest` and `/fw.zip` on a local port; returns the base URL.
    async fn serve(manifest: String, zip: Vec<u8>) -> String {
        let router = Router::new()
            .route("/manifest", get(move || async move { manifest }))
            .route("/fw.zip", get(move || async move { zip }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{address}")
    }

    #[test]
    fn the_newest_firmware_the_app_can_run_is_picked() {
        let manifest = manifest(
            "2026-10-07T00:00:00Z",
            vec![firmware("0.5.0", "0.5.0", "u", "s"), firmware("0.4.1", "0.4.0", "u", "s"), firmware("0.4.0", "0.3.2", "u", "s")],
        );
        let pick = |app: &str| pick_firmware(&manifest, &Version::parse(app).unwrap()).map(|f| f.download.version.clone());
        assert_eq!(pick("0.5.0").as_deref(), Some("0.5.0"));
        assert_eq!(pick("0.4.3").as_deref(), Some("0.4.1"), "an App too old for the newest still finds one");
        assert_eq!(pick("0.3.2").as_deref(), Some("0.4.0"));
        assert_eq!(pick("0.3.1"), None);
    }

    #[test]
    fn a_box_is_offered_newer_unversioned_or_dirty_firmware_but_never_a_downgrade() {
        assert!(offer_to_box("0.4.0", &box_running(Some("0.3.2"), "v0.3.2 2026-10-07 12:00")));
        assert!(!offer_to_box("0.4.0", &box_running(Some("0.4.0"), "firmware-v0.4.0 2026-10-07 12:00")));
        assert!(!offer_to_box("0.4.0", &box_running(Some("0.5.0"), "v0.5.0 2026-10-07 12:00")));
        assert!(offer_to_box("0.4.0", &box_running(None, "v0.3.2-33-gd6b1212 2026-10-06 18:54")), "firmware from before versions");
        assert!(offer_to_box("0.4.0", &box_running(Some("0.4.0"), "v0.3.2-43-g5541f3e-dirty 2026-10-07 12:11")));
        assert!(!offer_to_box("0.4.0", &box_running(Some("0.5.0"), "v0.5.0-dirty 2026-10-07 12:11")), "a dirty newer build is not downgraded");
        let devkit = DeviceState { unsupported_board: Some("goouuu-s3-spi".to_owned()), ..box_running(None, "685118c-dirty 2026-09-17 15:56") };
        assert!(!offer_to_box("0.4.0", &devkit), "released firmware is built for the box, not the breadboard devkit");
    }

    #[test]
    fn status_offers_only_what_is_newer_and_flags_an_unsupported_app() {
        let (_, public) = keys();
        let mut updates = Updates::new(Some(Source { url: "u".to_owned(), key: public }), None);
        let mut fresh = manifest("2026-10-07T00:00:00Z", vec![firmware("0.4.0", "0.0.0", "u", "s")]);
        fresh.min_supported_app = "99.0.0".to_owned();
        updates.accept("", fresh).unwrap();
        let status = updates.status(&box_running(Some("0.3.2"), "b"), true);
        assert!(status.enabled);
        if platform().is_some() {
            assert_eq!(status.app.map(|app| app.version).as_deref(), Some("99.0.0"));
        }
        assert!(status.unsupported_app);
        let firmware = status.firmware.unwrap();
        assert_eq!(firmware.version, "0.4.0");
        assert!(firmware.newer_than_box);
        assert_eq!(firmware.directory, None, "not downloaded");
        let unplugged = updates.status(&DeviceState::default(), false);
        assert!(!unplugged.enabled);
        assert!(!unplugged.firmware.unwrap().newer_than_box, "no box, nothing to offer it");
    }

    #[test]
    fn an_older_manifest_never_replaces_a_newer_one() {
        let mut updates = Updates::new(None, None);
        updates.accept("", manifest("2026-10-07T00:00:00Z", vec![firmware("0.4.0", "0.0.0", "u", "s")])).unwrap();
        let replayed = updates.accept("", manifest("2026-10-01T00:00:00Z", vec![firmware("0.3.9", "0.0.0", "u", "s")]));
        assert!(replayed.unwrap_err().contains("older"));
        assert_eq!(updates.manifest.as_ref().unwrap().firmware[0].download.version, "0.4.0");
    }

    #[test]
    fn the_accepted_manifest_is_kept_and_read_back_on_the_next_start() {
        let dir = temp_dir("keep");
        let (private, public) = keys();
        let source = Source { url: "u".to_owned(), key: public };
        let manifest = manifest("2026-10-07T00:00:00Z", vec![firmware("0.4.0", "0.0.0", "u", "s")]);
        let mut updates = Updates::new(Some(source.clone()), Some(dir.clone()));
        let wanted = updates.accept(&signed_text(&manifest, &private), manifest.clone()).unwrap();
        assert_eq!(wanted.map(|f| f.download.version).as_deref(), Some("0.4.0"), "not on disk yet");
        let restarted = Updates::new(Some(source), Some(dir.clone()));
        assert_eq!(restarted.manifest, Some(manifest));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_check_fetches_verifies_and_downloads_the_offered_firmware() {
        let zip = firmware_zip(true);
        let sha256 = hex(&Sha256::digest(&zip));
        let (private, public) = keys();
        // The base URL is only known once the server runs, and the manifest names the zip by URL.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let manifest = manifest("2026-10-07T00:00:00Z", vec![firmware("0.4.0", "0.0.0", &format!("{base}/fw.zip"), &sha256)]);
        let text = signed_text(&manifest, &private);
        let router = Router::new()
            .route("/manifest", get(move || async move { text }))
            .route("/fw.zip", get(move || async move { zip }));
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

        let source = Source { url: format!("{base}/manifest"), key: public };
        let (text, fetched) = tokio::task::spawn_blocking(move || fetch(&source, "0.3.2", Some("0.3.2"))).await.unwrap().unwrap();
        assert_eq!(fetched, manifest);
        assert!(text.contains("signature"));

        let root = temp_dir("download");
        std::fs::create_dir_all(root.join("0.3.9")).unwrap();
        let offered = fetched.firmware[0].clone();
        let dir = tokio::task::spawn_blocking({
            let root = root.clone();
            move || download(&offered, &root)
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(dir, root.join("0.4.0"));
        assert!(unpacked(&dir));
        assert_eq!(std::fs::read_to_string(dir.join("vibebuddy-fw.bin")).unwrap(), "firmware/vibebuddy-fw.bin");
        assert!(dir.join("version.txt").is_file());
        assert!(!dir.join("LICENSE").exists(), "only the firmware's own files are unpacked");
        assert!(!root.join("0.3.9").exists(), "older firmware is dropped");
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_manifest_signed_with_another_key_is_refused() {
        let (private, _) = keys();
        let (_, trusted) = keys();
        let base = serve(signed_text(&manifest("2026-10-07T00:00:00Z", vec![]), &private), Vec::new()).await;
        let source = Source { url: format!("{base}/manifest"), key: trusted };
        let error = tokio::task::spawn_blocking(move || fetch(&source, "0.3.2", None)).await.unwrap().unwrap_err();
        assert!(error.contains("signature"), "{error}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_download_that_does_not_match_its_sha256_is_refused() {
        let base = serve(String::new(), firmware_zip(true)).await;
        let root = temp_dir("mismatch");
        let offered = firmware("0.4.0", "0.0.0", &format!("{base}/fw.zip"), &"0".repeat(64));
        let error = tokio::task::spawn_blocking({
            let root = root.clone();
            move || download(&offered, &root)
        })
        .await
        .unwrap()
        .unwrap_err();
        assert!(error.contains("sha256"), "{error}");
        assert!(!root.join("0.4.0").exists());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_zip_without_the_app_image_leaves_nothing_behind() {
        let zip = firmware_zip(false);
        let sha256 = hex(&Sha256::digest(&zip));
        let base = serve(String::new(), zip).await;
        let root = temp_dir("incomplete");
        let offered = firmware("0.4.0", "0.0.0", &format!("{base}/fw.zip"), &sha256);
        let error = tokio::task::spawn_blocking({
            let root = root.clone();
            move || download(&offered, &root)
        })
        .await
        .unwrap()
        .unwrap_err();
        assert!(error.contains("vibebuddy-fw.bin"), "{error}");
        assert!(!root.join("0.4.0").exists() && !root.join(".0.4.0.partial").exists());
        let _ = std::fs::remove_dir_all(root);
    }
}
