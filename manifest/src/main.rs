//! CI's tool for the update manifest:
//!
//!   vibebuddy-manifest keygen <private-key-file>      writes a new private key (mode 600), prints the public key
//!   vibebuddy-manifest build <releases.json> <out>    builds and signs; the key comes from MANIFEST_SIGNING_KEY
//!   vibebuddy-manifest verify <public-key-file> <manifest>
//!
//! `releases.json` is `gh api --paginate --slurp repos/{owner}/{repo}/releases`. `build` runs from the repo root:
//! it reads the notes in `docs/releases` and `min_supported_app` from `Cargo.toml`.

use std::path::Path;
use std::process::ExitCode;

use vibebuddy_manifest::build::{self, Release};
use vibebuddy_manifest::{Signed, generate_key, sign, signing_key, verify, verifying_key};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        ["keygen", private] => keygen(Path::new(private)),
        ["build", releases, out] => build(Path::new(releases), Path::new(out)),
        ["verify", public, manifest] => check(Path::new(public), Path::new(manifest)),
        _ => Err("usage: vibebuddy-manifest keygen <private-key-file> | build <releases.json> <out> | verify <public-key-file> <manifest>".to_owned()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("vibebuddy-manifest: {message}");
            ExitCode::FAILURE
        }
    }
}

fn keygen(private: &Path) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let (secret, public) = generate_key().map_err(|error| error.to_string())?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(private)
        .map_err(|error| format!("{}: {error}", private.display()))?;
    writeln!(file, "{secret}").map_err(|error| error.to_string())?;
    println!("{public}");
    Ok(())
}

fn build(releases: &Path, out: &Path) -> Result<(), String> {
    let key = std::env::var("MANIFEST_SIGNING_KEY").map_err(|_| "MANIFEST_SIGNING_KEY is not set".to_owned())?;
    let key = signing_key(&key).map_err(|error| error.to_string())?;
    let releases = read_releases(releases)?;
    let min_supported_app = min_supported_app(Path::new("Cargo.toml"))?;
    let generated_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let notes = |name: &str| std::fs::read_to_string(Path::new("docs/releases").join(name)).ok();
    let manifest = build::manifest(&releases, notes, &min_supported_app, &generated_at)?;
    let signed = sign(&manifest, &key);
    std::fs::write(out, serde_json::to_string_pretty(&signed).expect("serializes")).map_err(|error| format!("{}: {error}", out.display()))?;
    let firmware: Vec<&str> = manifest.firmware.iter().map(|firmware| firmware.download.version.as_str()).collect();
    let apps: Vec<String> = manifest.app.iter().map(|(platform, app)| format!("{platform} {}", app.version)).collect();
    println!("apps: {}; firmware: {}", apps.join(", "), if firmware.is_empty() { "none".to_owned() } else { firmware.join(", ") });
    Ok(())
}

fn check(public: &Path, manifest: &Path) -> Result<(), String> {
    let key = std::fs::read_to_string(public).map_err(|error| format!("{}: {error}", public.display()))?;
    let key = verifying_key(&key).map_err(|error| error.to_string())?;
    let text = std::fs::read_to_string(manifest).map_err(|error| format!("{}: {error}", manifest.display()))?;
    let signed: Signed = serde_json::from_str(&text).map_err(|error| error.to_string())?;
    verify(&signed, &key).map_err(|error| error.to_string())?;
    println!("signature ok");
    Ok(())
}

/// `--paginate --slurp` gives one array per page; a single page is accepted as is.
fn read_releases(path: &Path) -> Result<Vec<Release>, String> {
    let text = std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    if let Ok(pages) = serde_json::from_str::<Vec<Vec<Release>>>(&text) {
        return Ok(pages.into_iter().flatten().collect());
    }
    serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))
}

fn min_supported_app(cargo: &Path) -> Result<String, String> {
    let text = std::fs::read_to_string(cargo).map_err(|error| format!("{}: {error}", cargo.display()))?;
    let table: toml::Table = toml::from_str(&text).map_err(|error| format!("{}: {error}", cargo.display()))?;
    table
        .get("workspace")
        .and_then(|workspace| workspace.get("metadata"))
        .and_then(|metadata| metadata.get("vibebuddy"))
        .and_then(|vibebuddy| vibebuddy.get("min_supported_app"))
        .and_then(|version| version.as_str())
        .map(str::to_owned)
        .ok_or_else(|| "Cargo.toml has no [workspace.metadata.vibebuddy] min_supported_app".to_owned())
}
