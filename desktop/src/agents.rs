//! The Agents tab's view of each agent's hook config, the counterpart of the Mac's `HookInstaller`. The rules for
//! merging into other programs' config live in `vibebuddy-hook` (installed next to this binary) and only there: this
//! module asks it for each agent's state (`status`), for the change before writing it (`plan`), and to write it.

use std::path::PathBuf;

use serde::Deserialize;

/// One agent, as `vibebuddy-hook status` reports it.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Setup {
    /// The hook's argument for it: codex, claude, opencode, copilot, pi.
    pub agent: String,
    pub name: String,
    /// Its config directory exists, so it has run on this machine.
    pub present: bool,
    pub installed: bool,
    pub config: PathBuf,
    /// When the config file was last written (RFC 3339).
    pub modified: Option<String>,
}

/// What Connect, Repair or Remove would write, shown before writing it.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub agent: String,
    pub installing: bool,
    pub config: PathBuf,
    /// `+ …` and `- …` lines; empty when nothing would change.
    pub lines: Vec<String>,
}

/// Whether Codex is running the config, judged as on the Mac (`HookConfig.codexTrustHint`): Codex silently disables
/// hooks that changed until they're trusted again, so a config written after the last Codex event is suspect.
#[derive(Clone, Debug, PartialEq)]
pub enum CodexTrust {
    WaitingFirstEvent,
    /// The config changed at this time and no event has arrived since.
    ChangedSinceLastEvent(String),
    Trusted,
}

pub fn codex_trust(modified: Option<&str>, last_event: Option<&str>) -> CodexTrust {
    let parse = |time: &str| chrono::DateTime::parse_from_rfc3339(time).ok();
    let Some(last) = last_event.and_then(parse) else { return CodexTrust::WaitingFirstEvent };
    match modified.filter(|modified| parse(modified).is_some_and(|modified| modified > last)) {
        Some(modified) => CodexTrust::ChangedSinceLastEvent(modified.to_owned()),
        None => CodexTrust::Trusted,
    }
}

pub async fn status() -> Result<Vec<Setup>, String> {
    let output = hook(&["status"]).await?;
    serde_json::from_str(&output).map_err(|error| format!("unexpected answer from vibebuddy-hook status: {error}"))
}

pub async fn plan(agent: String, installing: bool) -> Result<Plan, String> {
    #[derive(Deserialize)]
    struct Answer {
        config: PathBuf,
        lines: Vec<String>,
    }
    let action = if installing { "install" } else { "uninstall" };
    let output = hook(&["plan", action, &agent]).await?;
    let answer: Answer =
        serde_json::from_str(&output).map_err(|error| format!("unexpected answer from vibebuddy-hook plan: {error}"))?;
    Ok(Plan { agent, installing, config: answer.config, lines: answer.lines })
}

/// Writes what the plan showed. The hook works it out again from the file as it is now, so a file changed in the
/// meantime is never overwritten with a stale copy.
pub async fn apply(plan: Plan) -> Result<String, String> {
    hook(&[if plan.installing { "install" } else { "uninstall" }, &plan.agent]).await
}

async fn hook(args: &[&str]) -> Result<String, String> {
    let hook = std::env::current_exe().map_err(|error| error.to_string())?.with_file_name("vibebuddy-hook");
    let output = tokio::process::Command::new(&hook)
        .args(args)
        .output()
        .await
        .map_err(|error| format!("cannot run {}: {error}", hook.display()))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hooks_status_parses() {
        let json = r#"[{"agent":"codex","name":"Codex","present":true,"installed":true,
            "config":"/home/me/.codex/hooks.json","modified":"2026-10-09T10:00:00+08:00"},
            {"agent":"pi","name":"Pi","present":false,"installed":false,"config":"/home/me/.pi/agent/extensions/vibebuddy.js","modified":null}]"#;
        let setups: Vec<Setup> = serde_json::from_str(json).expect("status");
        assert_eq!(setups.len(), 2);
        assert!(setups[0].installed && !setups[1].present);
    }

    #[test]
    fn codex_trust_follows_the_mac() {
        let written = "2026-10-09T10:00:00+08:00";
        assert_eq!(codex_trust(Some(written), None), CodexTrust::WaitingFirstEvent);
        assert_eq!(codex_trust(Some(written), Some("2026-10-09T10:05:00+08:00")), CodexTrust::Trusted);
        // A different offset for the same instant ordering still compares by time, not text.
        assert_eq!(
            codex_trust(Some(written), Some("2026-10-09T01:59:00Z")),
            CodexTrust::ChangedSinceLastEvent(written.to_owned())
        );
        assert_eq!(codex_trust(None, Some("2026-10-09T01:59:00Z")), CodexTrust::Trusted);
    }
}
