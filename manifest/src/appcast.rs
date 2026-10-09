//! Sparkle's appcast for the macOS App (ADR-0010): one item, the App the manifest lists for `macos-arm64`, with its
//! DMG signed by the separate Sparkle key. Sparkle checks that EdDSA signature against `SUPublicEDKey` before it swaps
//! the bundle. Apps older than `min_supported_app` see the item as critical, so Sparkle offers no "Skip".

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};

use crate::{Download, Manifest};

pub const PLATFORM: &str = "macos-arm64";
/// LSMinimumSystemVersion in app/Info.plist.
const MINIMUM_SYSTEM: &str = "14.0";

/// `dmg` is the exact file the manifest's URL serves: a notarized DMG changes when its ticket is stapled, so it is
/// signed as published.
pub fn appcast(manifest: &Manifest, dmg: &[u8], key: &SigningKey) -> Result<String, String> {
    let app = manifest.app.get(PLATFORM).ok_or_else(|| format!("the manifest has no {PLATFORM} App"))?;
    // Sign only the file the manifest vouches for: what Sparkle downloads is what users get from the same URL.
    let digest: String = Sha256::digest(dmg).iter().map(|byte| format!("{byte:02x}")).collect();
    if !digest.eq_ignore_ascii_case(&app.sha256) {
        return Err(format!("the DMG has sha256 {digest}, the manifest says {}", app.sha256));
    }
    let signature = STANDARD.encode(key.sign(dmg).to_bytes());
    Ok(render(app, &manifest.min_supported_app, &signature, dmg.len()))
}

fn render(app: &Download, min_supported_app: &str, signature: &str, length: usize) -> String {
    // Release builds' CFBundleVersion is `git describe` at the tag, `vX.Y.Z`; Sparkle compares against that.
    let version = format!("v{}", app.version);
    let mut descriptions = String::new();
    for (language, notes) in &app.notes {
        // Sparkle shows the description matching the user's language, English otherwise.
        let lang = if language == "en" { String::new() } else { format!(" xml:lang=\"{}\"", escape(language)) };
        descriptions.push_str(&format!("      <description{lang}><![CDATA[{}]]></description>\n", html(notes)));
    }
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
  <channel>
    <title>VibeBuddy</title>
    <item>
      <title>VibeBuddy {short}</title>
      <sparkle:version>{version}</sparkle:version>
      <sparkle:shortVersionString>{short}</sparkle:shortVersionString>
      <sparkle:minimumSystemVersion>{MINIMUM_SYSTEM}</sparkle:minimumSystemVersion>
      <sparkle:criticalUpdate sparkle:version="v{minimum}"></sparkle:criticalUpdate>
{descriptions}      <enclosure url="{url}" length="{length}" type="application/octet-stream" sparkle:edSignature="{signature}"/>
    </item>
  </channel>
</rss>
"#,
        short = escape(&app.version),
        version = escape(&version),
        minimum = escape(min_supported_app),
        url = escape(&app.url),
    )
}

/// The release notes are a few Markdown bullets under a heading; Sparkle shows HTML. Only that much is converted.
fn html(markdown: &str) -> String {
    let mut out = String::new();
    let mut in_list = false;
    for line in markdown.lines().map(str::trim) {
        let item = line.strip_prefix("- ");
        if item.is_none() && in_list {
            out.push_str("</ul>");
            in_list = false;
        }
        if let Some(item) = item {
            if !in_list {
                out.push_str("<ul>");
                in_list = true;
            }
            out.push_str(&format!("<li>{}</li>", inline(item)));
        } else if let Some(heading) = line.strip_prefix('#').map(|rest| rest.trim_start_matches('#').trim()) {
            out.push_str(&format!("<h3>{}</h3>", inline(heading)));
        } else if !line.is_empty() {
            out.push_str(&format!("<p>{}</p>", inline(line)));
        }
    }
    if in_list {
        out.push_str("</ul>");
    }
    out
}

/// `code` spans become <code>; everything else is escaped.
fn inline(text: &str) -> String {
    text.split('`')
        .enumerate()
        .map(|(index, part)| if index % 2 == 1 { format!("<code>{}</code>", escape(part)) } else { escape(part) })
        .collect()
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use ed25519_dalek::{Signature, Verifier};

    use super::*;
    use crate::{Notes, SCHEMA, generate_key, signing_key, verifying_key};

    const DMG: &[u8] = b"not really a dmg";

    fn manifest() -> Manifest {
        let notes = Notes::from([
            ("en".to_owned(), "## What's new\n\n- Faster `Check now`\n- Fixes <things>".to_owned()),
            ("zh-Hans".to_owned(), "## 新功能\n\n- 更快".to_owned()),
        ]);
        let app = Download {
            version: "0.4.0".to_owned(),
            url: "https://github.com/x/y/releases/download/v0.4.0/VibeBuddy-v0.4.0-arm64.dmg".to_owned(),
            sha256: Sha256::digest(DMG).iter().map(|byte| format!("{byte:02x}")).collect(),
            notes,
        };
        Manifest {
            schema: SCHEMA,
            generated_at: "2026-10-07T00:00:00Z".to_owned(),
            min_supported_app: "0.3.2".to_owned(),
            app: BTreeMap::from([(PLATFORM.to_owned(), app)]),
            firmware: vec![],
        }
    }

    #[test]
    fn the_dmg_signature_verifies_with_the_public_key() {
        let (private, public) = generate_key().unwrap();
        let dmg = DMG;
        let xml = appcast(&manifest(), dmg, &signing_key(&private).unwrap()).unwrap();
        let signature = xml.split("sparkle:edSignature=\"").nth(1).unwrap().split('"').next().unwrap();
        let signature = Signature::from_slice(&STANDARD.decode(signature).unwrap()).unwrap();
        assert!(verifying_key(&public).unwrap().verify(dmg, &signature).is_ok());
        assert!(xml.contains(&format!("length=\"{}\"", dmg.len())));
    }

    #[test]
    fn the_item_carries_versions_the_way_sparkle_compares_them() {
        let (private, _) = generate_key().unwrap();
        let xml = appcast(&manifest(), DMG, &signing_key(&private).unwrap()).unwrap();
        assert!(xml.contains("<sparkle:version>v0.4.0</sparkle:version>"));
        assert!(xml.contains("<sparkle:shortVersionString>0.4.0</sparkle:shortVersionString>"));
        assert!(xml.contains(r#"<sparkle:criticalUpdate sparkle:version="v0.3.2">"#), "older than min_supported_app can't skip");
        assert!(xml.contains("url=\"https://github.com/x/y/releases/download/v0.4.0/VibeBuddy-v0.4.0-arm64.dmg\""));
    }

    #[test]
    fn notes_become_html_in_each_language() {
        let (private, _) = generate_key().unwrap();
        let xml = appcast(&manifest(), DMG, &signing_key(&private).unwrap()).unwrap();
        assert!(xml.contains("<description><![CDATA[<h3>What's new</h3><ul><li>Faster <code>Check now</code></li><li>Fixes &lt;things&gt;</li></ul>]]></description>"));
        assert!(xml.contains(r#"<description xml:lang="zh-Hans"><![CDATA[<h3>新功能</h3><ul><li>更快</li></ul>]]></description>"#));
    }

    #[test]
    fn a_manifest_without_a_mac_app_has_no_appcast() {
        let (private, _) = generate_key().unwrap();
        let mut manifest = manifest();
        manifest.app.clear();
        assert!(appcast(&manifest, DMG, &signing_key(&private).unwrap()).unwrap_err().contains(PLATFORM));
    }

    #[test]
    fn a_dmg_the_manifest_doesnt_vouch_for_is_not_signed() {
        let (private, _) = generate_key().unwrap();
        assert!(appcast(&manifest(), b"another file", &signing_key(&private).unwrap()).unwrap_err().contains("sha256"));
    }
}
