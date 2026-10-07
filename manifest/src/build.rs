//! Builds the manifest from what has been released: the GitHub releases (versions, assets and their sha256) and the
//! release notes checked in under `docs/releases`. Run on every release of either kind, so each keeps the other's entry.

use std::collections::BTreeMap;

use semver::Version;
use serde::Deserialize;

use crate::{Download, Firmware, Manifest, Notes, SCHEMA};

/// The parts of a GitHub release (`GET /repos/{owner}/{repo}/releases`) the manifest needs.
#[derive(Clone, Debug, Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    pub assets: Vec<Asset>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
    /// `sha256:<hex>`, which GitHub computes for every uploaded asset.
    pub digest: Option<String>,
}

/// The App's package per platform: release-app.yml names it `VibeBuddy-<tag><suffix>`.
const APP_ASSETS: [(&str, &str); 2] = [("macos-arm64", "-arm64.dmg"), ("linux-x86_64", "-linux-x86_64.tar.gz")];

const FIRMWARE_TAG: &str = "firmware-v";

/// `notes` reads a file under `docs/releases` by name: `<tag>.md` (English, required) and `<tag>.zh-Hans.md`.
/// A firmware release's English notes start with front matter naming the oldest App it runs with:
///
/// ```text
/// ---
/// min_app: 0.4.0
/// ---
/// ```
pub fn manifest(releases: &[Release], notes: impl Fn(&str) -> Option<String>, min_supported_app: &str, generated_at: &str) -> Result<Manifest, String> {
    let published: Vec<&Release> = releases.iter().filter(|release| !release.draft && !release.prerelease).collect();

    let mut apps: Vec<(Version, &Release)> = published
        .iter()
        .filter_map(|release| Some((version(release.tag_name.strip_prefix('v')?)?, *release)))
        .collect();
    apps.sort_by(|a, b| b.0.cmp(&a.0));
    let mut app = BTreeMap::new();
    for (platform, suffix) in APP_ASSETS {
        // The newest release that has this platform's package, so a release without one doesn't hide the last that had it.
        let found = apps.iter().find_map(|(version, release)| Some((version, release, asset(release, &format!("VibeBuddy-{}{suffix}", release.tag_name))?)));
        if let Some((version, release, asset)) = found {
            let (notes, _) = read_notes(&notes, &release.tag_name)?;
            app.insert(platform.to_owned(), download(version, asset, notes)?);
        }
    }

    let mut firmware: Vec<(Version, Firmware)> = Vec::new();
    for release in &published {
        let Some(version) = release.tag_name.strip_prefix(FIRMWARE_TAG).and_then(version) else { continue };
        let tag = &release.tag_name;
        let zip = asset(release, &format!("VibeBuddy-firmware-v{version}.zip")).ok_or_else(|| format!("{tag} has no firmware zip"))?;
        let (notes, min_app) = read_notes(&notes, tag)?;
        let min_app = min_app.ok_or_else(|| format!("docs/releases/{tag}.md doesn't say min_app"))?;
        Version::parse(&min_app).map_err(|error| format!("{tag}: min_app {min_app}: {error}"))?;
        firmware.push((version.clone(), Firmware { min_app, download: download(&version, zip, notes)? }));
    }
    firmware.sort_by(|a, b| b.0.cmp(&a.0));

    Version::parse(min_supported_app).map_err(|error| format!("min_supported_app {min_supported_app}: {error}"))?;
    Ok(Manifest {
        schema: SCHEMA,
        generated_at: generated_at.to_owned(),
        min_supported_app: min_supported_app.to_owned(),
        app,
        firmware: firmware.into_iter().map(|(_, firmware)| firmware).collect(),
    })
}

fn version(text: &str) -> Option<Version> {
    Version::parse(text).ok().filter(|version| version.pre.is_empty())
}

fn asset<'a>(release: &'a Release, name: &str) -> Option<&'a Asset> {
    release.assets.iter().find(|asset| asset.name == name)
}

fn download(version: &Version, asset: &Asset, notes: Notes) -> Result<Download, String> {
    let sha256 = asset
        .digest
        .as_deref()
        .and_then(|digest| digest.strip_prefix("sha256:"))
        .ok_or_else(|| format!("{} has no sha256 digest", asset.name))?;
    Ok(Download { version: version.to_string(), url: asset.browser_download_url.clone(), sha256: sha256.to_owned(), notes })
}

/// The notes in each language, plus `min_app` from the English file's front matter.
fn read_notes(notes: &impl Fn(&str) -> Option<String>, tag: &str) -> Result<(Notes, Option<String>), String> {
    let english = notes(&format!("{tag}.md")).ok_or_else(|| format!("missing docs/releases/{tag}.md"))?;
    let (front, english) = front_matter(&english);
    let min_app = front.lines().find_map(|line| line.strip_prefix("min_app:")).map(|value| value.trim().to_owned());
    let mut result = Notes::from([("en".to_owned(), english.trim().to_owned())]);
    if let Some(chinese) = notes(&format!("{tag}.zh-Hans.md")) {
        result.insert("zh-Hans".to_owned(), front_matter(&chinese).1.trim().to_owned());
    }
    Ok((result, min_app))
}

/// Splits `---\n…\n---\n` off the top; text without it has empty front matter.
fn front_matter(text: &str) -> (&str, &str) {
    text.strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---\n"))
        .unwrap_or(("", text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, assets: &[&str]) -> Release {
        Release {
            tag_name: tag.to_owned(),
            draft: false,
            prerelease: false,
            assets: assets
                .iter()
                .map(|name| Asset {
                    name: (*name).to_owned(),
                    browser_download_url: format!("https://github.com/x/y/releases/download/{tag}/{name}"),
                    digest: Some(format!("sha256:{}", name.len())),
                })
                .collect(),
        }
    }

    fn notes(name: &str) -> Option<String> {
        match name {
            "v0.4.0.md" => Some("## What's new\n\n- New\n".to_owned()),
            "v0.4.0.zh-Hans.md" => Some("## 新功能\n\n- 新\n".to_owned()),
            "v0.3.2.md" => Some("- Old\n".to_owned()),
            "firmware-v0.3.3.md" => Some("---\nmin_app: 0.3.2\n---\n- Firmware fix\n".to_owned()),
            "firmware-v0.4.0.md" => Some("---\nmin_app: 0.4.0\n---\n- Needs the new App\n".to_owned()),
            _ => None,
        }
    }

    fn releases() -> Vec<Release> {
        vec![
            release("firmware-v0.4.0", &["VibeBuddy-firmware-v0.4.0.zip"]),
            release("v0.4.0", &["VibeBuddy-v0.4.0-arm64.dmg"]),
            release("firmware-v0.3.3", &["VibeBuddy-firmware-v0.3.3.zip"]),
            release("v0.3.2", &["VibeBuddy-v0.3.2-arm64.dmg", "VibeBuddy-v0.3.2-linux-x86_64.tar.gz", "VibeBuddy-firmware-v0.3.2.zip"]),
        ]
    }

    #[test]
    fn the_latest_app_per_platform_and_every_firmware_are_listed() {
        let manifest = manifest(&releases(), notes, "0.3.2", "2026-10-07T12:00:00Z").unwrap();
        assert_eq!(manifest.app["macos-arm64"].version, "0.4.0");
        assert_eq!(manifest.app["macos-arm64"].url, "https://github.com/x/y/releases/download/v0.4.0/VibeBuddy-v0.4.0-arm64.dmg");
        // v0.4.0 shipped no Linux package; the last one that did is still offered.
        assert_eq!(manifest.app["linux-x86_64"].version, "0.3.2");
        let firmware: Vec<(&str, &str)> = manifest.firmware.iter().map(|f| (f.download.version.as_str(), f.min_app.as_str())).collect();
        // The firmware zips bundled in App releases before ADR-0010 aren't firmware releases.
        assert_eq!(firmware, [("0.4.0", "0.4.0"), ("0.3.3", "0.3.2")]);
    }

    #[test]
    fn notes_come_in_both_languages_without_front_matter() {
        let manifest = manifest(&releases(), notes, "0.3.2", "t").unwrap();
        let app = &manifest.app["macos-arm64"].notes;
        assert_eq!(app["en"], "## What's new\n\n- New");
        assert_eq!(app["zh-Hans"], "## 新功能\n\n- 新");
        let firmware = &manifest.firmware[1].download.notes;
        assert_eq!(firmware["en"], "- Firmware fix");
        assert!(!firmware.contains_key("zh-Hans"), "English is the fallback");
    }

    #[test]
    fn the_digest_becomes_the_sha256() {
        let manifest = manifest(&releases(), notes, "0.3.2", "t").unwrap();
        assert_eq!(manifest.app["macos-arm64"].sha256, "26");
    }

    #[test]
    fn drafts_and_prereleases_are_left_out() {
        let mut releases = releases();
        releases[0].prerelease = true;
        releases[1].draft = true;
        let manifest = manifest(&releases, notes, "0.3.2", "t").unwrap();
        assert_eq!(manifest.app["macos-arm64"].version, "0.3.2");
        assert_eq!(manifest.firmware.len(), 1);
    }

    #[test]
    fn firmware_without_min_app_stops_the_build() {
        let missing = |name: &str| if name == "firmware-v0.3.3.md" { Some("- No front matter\n".to_owned()) } else { notes(name) };
        let error = manifest(&releases(), missing, "0.3.2", "t").unwrap_err();
        assert!(error.contains("min_app"), "{error}");
    }

    #[test]
    fn a_release_without_notes_stops_the_build() {
        let error = manifest(&releases(), |name| if name == "v0.4.0.md" { None } else { notes(name) }, "0.3.2", "t").unwrap_err();
        assert!(error.contains("docs/releases/v0.4.0.md"), "{error}");
    }

    #[test]
    fn an_asset_without_a_digest_stops_the_build() {
        let mut releases = releases();
        releases[2].assets[0].digest = None;
        assert!(manifest(&releases, notes, "0.3.2", "t").unwrap_err().contains("sha256"));
    }
}
