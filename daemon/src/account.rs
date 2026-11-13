//! The person's VibeBuddy account (ADR-0018, docs/api.md): signing in through the browser, keeping the session, and
//! signing out or deleting the account. The App only opens the URL the daemon gives it; the browser comes back to
//! the daemon's own `/v1/account/callback`, so the session never passes through the App.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::watch;
use tracing::{info, warn};

/// In the config directory, beside `identity`; mode 0600.
pub const ACCOUNT_FILE: &str = "account";
/// As long as the service keeps a sign-in open.
const PENDING_FOR: Duration = Duration::from_secs(15 * 60);
const TIMEOUT: Duration = Duration::from_secs(20);

/// What `GET /v1/status` says about the account.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct AccountStatus {
    /// This build has a service to sign in to: a release has none until production opens (vibebuddy-cloud#11).
    pub available: bool,
    pub signed_in: bool,
    pub email: Option<String>,
    pub github_id: Option<u64>,
    /// Looked up on GitHub when signing in, for the App to show; the service keeps only the id.
    pub github_login: Option<String>,
    /// The account's plan (ADR-0023), read from the service at sign-in and when the App asks.
    pub plan: Option<Plan>,
    /// The voice credit left, in millionths of a US dollar (ADR-0024), read with the plan. None before the account
    /// first uses our voice service, and when the service couldn't read it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice_credit_micro_usd: Option<u64>,
    /// The one switch for the Paid plan (ADR-0024): the service's `upgrade_offered`, read with the plan. False before
    /// the first read, signed out, or when the service has no such field. While it is false, `POST
    /// /v1/account/upgrade` doesn't ask the service, and the Apps don't offer the Paid plan. Manage plan doesn't
    /// depend on it.
    pub upgrade_offered: bool,
}

/// A Stripe page on the service (docs/api.md), which the App opens in the browser.
#[derive(Clone, Copy, Debug)]
pub enum PlanPage {
    /// Checkout, to start the Paid plan.
    Checkout,
    /// The Customer Portal, to change the card or cancel.
    Portal,
}

/// As the service's `GET /v1/account` says it (docs/api.md): `{"name":"free"}`, or `paid` with the billing
/// period's end.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Plan {
    /// `free` or `paid`.
    pub name: String,
    /// When the period ends and the plan renews, RFC 3339.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period_end: Option<String>,
}

/// What the last read of the account said, besides who it is.
#[derive(Default)]
struct Details {
    plan: Option<Plan>,
    voice_credit_micro_usd: Option<u64>,
    upgrade_offered: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Saved {
    token: String,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    github_id: Option<u64>,
    #[serde(default)]
    github_login: Option<String>,
}

struct Pending {
    verifier: String,
    state: String,
    started: Instant,
}

#[derive(Debug, PartialEq)]
pub enum AccountError {
    /// No service in this build, or no config directory to keep the session in.
    Unavailable,
    /// The callback didn't match a sign-in this daemon started.
    UnknownSignIn,
    /// The service refused or couldn't be reached; the message is for logs and the App.
    Service(String),
    NotSignedIn,
    /// The service said 409 with this reason: `already_paid` for Checkout, `no_subscription` for the portal.
    Conflict(String),
    /// The service doesn't offer the plan pages (a 404: Stripe isn't set up there), or the account says Upgrade isn't offered.
    NotOffered,
}

impl std::fmt::Display for AccountError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable => write!(f, "signing in isn't available in this build"),
            Self::UnknownSignIn => write!(f, "this sign-in wasn't started here, or has expired"),
            Self::Service(message) => write!(f, "{message}"),
            Self::NotSignedIn => write!(f, "not signed in"),
            Self::Conflict(reason) => write!(f, "the service refused: {reason}"),
            Self::NotOffered => write!(f, "the plan isn't offered here"),
        }
    }
}

pub struct Account {
    /// The service's base URL (`synthesis::service_base`); None when it is switched off.
    base: Option<String>,
    file: Option<PathBuf>,
    redirect_uri: String,
    github_api: String,
    saved: Mutex<Option<Saved>>,
    pending: Mutex<Option<Pending>>,
    details: Mutex<Details>,
    /// Bumped whenever [`AccountStatus::plan`] or [`AccountStatus::voice_credit_micro_usd`] is read, for the status
    /// stream.
    details_changed: watch::Sender<u64>,
    /// Whether signed in, for whoever must follow it: the status stream and the box's offer of voice.
    signed_in: watch::Sender<bool>,
    /// Bumped on signing in or out and on each read that finds the Paid plan: what a computer the Relay turned
    /// away for its account waits for before trying again.
    relay_chance: watch::Sender<u64>,
}

impl Account {
    /// `port` is the daemon's own, which the browser comes back to.
    pub fn new(base: Option<String>, config_dir: Option<&Path>, port: u16) -> Self {
        let file = config_dir.map(|dir| dir.join(ACCOUNT_FILE));
        let saved = file.as_deref().and_then(load);
        let (signed_in, _) = watch::channel(saved.is_some());
        let (details_changed, _) = watch::channel(0);
        let (relay_chance, _) = watch::channel(0);
        Self {
            base,
            file,
            redirect_uri: format!("http://127.0.0.1:{port}/v1/account/callback"),
            github_api: "https://api.github.com".to_owned(),
            saved: Mutex::new(saved),
            pending: Mutex::new(None),
            details: Mutex::default(),
            details_changed,
            signed_in,
            relay_chance,
        }
    }

    /// Changes when signing in or out, or a read of the plan finds the Paid plan.
    pub fn relay_chance(&self) -> u64 {
        *self.relay_chance.borrow()
    }

    /// Changes whenever the plan or the voice credit is read.
    pub fn watch_details(&self) -> watch::Receiver<u64> {
        self.details_changed.subscribe()
    }

    /// The service's base URL, which our voice service is at too; None when it is switched off.
    pub fn base(&self) -> Option<String> {
        self.base.clone()
    }

    /// Changes each time the account is signed in or out, from here or by the service refusing the session.
    pub fn watch(&self) -> watch::Receiver<bool> {
        self.signed_in.subscribe()
    }

    #[cfg(test)]
    fn with_github_api(mut self, url: &str) -> Self {
        self.github_api = url.to_owned();
        self
    }

    pub fn status(&self) -> AccountStatus {
        let saved = self.saved.lock().unwrap().clone();
        let details = self.details.lock().unwrap();
        let signed_in = saved.is_some();
        AccountStatus {
            available: self.base.is_some() && self.file.is_some(),
            signed_in,
            email: saved.as_ref().and_then(|s| s.email.clone()),
            github_id: saved.as_ref().and_then(|s| s.github_id),
            github_login: saved.and_then(|s| s.github_login),
            plan: details.plan.clone().filter(|_| signed_in),
            voice_credit_micro_usd: details.voice_credit_micro_usd.filter(|_| signed_in),
            upgrade_offered: details.upgrade_offered && signed_in,
        }
    }

    /// Reads the plan and the voice credit from the service. Blocking.
    pub fn refresh(&self) -> Result<(), AccountError> {
        let token = self.token().ok_or(AccountError::NotSignedIn)?;
        let base = self.base.clone().ok_or(AccountError::Unavailable)?;
        let (status, text) = read(crate::speech::agent(TIMEOUT).get(&format!("{base}/v1/account")).header("Authorization", &format!("Bearer {token}")).call())?;
        if status == 401 {
            self.forget();
            return Err(AccountError::NotSignedIn);
        }
        if status != 200 {
            return Err(AccountError::Service(format!("the account couldn't be read: HTTP {status}")));
        }
        let account: serde_json::Value = serde_json::from_str(&text).map_err(|error| AccountError::Service(error.to_string()))?;
        self.take_details(&account);
        Ok(())
    }

    /// The URL of a Stripe page for this account, from the service's `POST /v1/plan/checkout` or `/v1/plan/portal`.
    /// Blocking.
    pub fn plan_page(&self, page: PlanPage) -> Result<String, AccountError> {
        let token = self.token().ok_or(AccountError::NotSignedIn)?;
        let base = self.base.clone().ok_or(AccountError::Unavailable)?;
        // The App opens the URL in the browser, so only Stripe's own page for what was asked goes through.
        let (path, host) = match page {
            PlanPage::Checkout => ("checkout", "checkout.stripe.com"),
            PlanPage::Portal => ("portal", "billing.stripe.com"),
        };
        let (status, text) = read(crate::speech::agent(TIMEOUT).post(&format!("{base}/v1/plan/{path}")).header("Authorization", &format!("Bearer {token}")).send_empty())?;
        let body = serde_json::from_str::<serde_json::Value>(&text).unwrap_or_default();
        match status {
            200 => body["url"]
                .as_str()
                .filter(|url| url.starts_with(&format!("https://{host}/")))
                .map(str::to_owned)
                .ok_or_else(|| AccountError::Service(format!("the service sent no {host} URL"))),
            401 => {
                self.forget();
                Err(AccountError::NotSignedIn)
            }
            404 => Err(AccountError::NotOffered),
            409 => Err(AccountError::Conflict(body["reason"].as_str().unwrap_or_default().to_owned())),
            _ => Err(AccountError::Service(format!("the service didn't open the page: HTTP {status}"))),
        }
    }

    /// The plan, the voice credit and whether Upgrade is offered, from a `GET /v1/account` answer (docs/api.md).
    fn take_details(&self, account: &serde_json::Value) {
        let plan = &account["plan"];
        let plan = plan["name"].as_str().map(|name| Plan { name: name.to_owned(), period_end: plan["period_end"].as_str().map(str::to_owned) });
        if plan.as_ref().is_some_and(|plan| plan.name == "paid") {
            self.relay_chance.send_modify(|n| *n += 1);
        }
        *self.details.lock().unwrap() = Details {
            plan,
            voice_credit_micro_usd: account["voice_credit_micro_usd"].as_u64(),
            upgrade_offered: account["upgrade_offered"].as_bool().unwrap_or(false),
        };
        self.details_changed.send_modify(|n| *n += 1);
    }

    /// The session for calls to the service, when signed in.
    pub fn token(&self) -> Option<String> {
        self.saved.lock().unwrap().as_ref().map(|saved| saved.token.clone())
    }

    /// Starts a sign-in and returns the URL for the App to open; a new one replaces any unfinished one.
    pub fn start(&self) -> Result<String, AccountError> {
        let (Some(base), Some(_)) = (&self.base, &self.file) else { return Err(AccountError::Unavailable) };
        let verifier = random_token();
        let state = random_token();
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let query = [
            ("redirect_uri", self.redirect_uri.as_str()),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("state", state.as_str()),
        ]
        .iter()
        .map(|(key, value)| format!("{key}={}", percent_encode(value)))
        .collect::<Vec<_>>()
        .join("&");
        *self.pending.lock().unwrap() = Some(Pending { verifier, state, started: Instant::now() });
        Ok(format!("{base}/v1/sign-in?{query}"))
    }

    /// The browser came back with `code` and `state`: exchange the code for a session and keep it. Blocking.
    pub fn finish(&self, code: &str, state: &str) -> Result<(), AccountError> {
        let base = self.base.clone().ok_or(AccountError::Unavailable)?;
        let verifier = {
            let mut pending = self.pending.lock().unwrap();
            match pending.take() {
                Some(p) if constant_time_eq(p.state.as_bytes(), state.as_bytes()) && p.started.elapsed() < PENDING_FOR => p.verifier,
                // A wrong or stale state leaves a live sign-in alone, so a stray request can't cancel it.
                other => {
                    *pending = other.filter(|p| p.started.elapsed() < PENDING_FOR);
                    return Err(AccountError::UnknownSignIn);
                }
            }
        };
        let agent = crate::speech::agent(TIMEOUT);
        let body = serde_json::json!({"code": code, "code_verifier": verifier, "redirect_uri": self.redirect_uri}).to_string();
        let (status, text) = read(agent.post(&format!("{base}/v1/session")).header("Content-Type", "application/json").send(body.as_bytes()))?;
        if status != 200 {
            return Err(AccountError::Service(format!("the service refused the sign-in: HTTP {status} {text}")));
        }
        let token = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|value| value["token"].as_str().map(str::to_owned))
            .ok_or_else(|| AccountError::Service("the service sent no session".to_owned()))?;

        let (status, text) = read(agent.get(&format!("{base}/v1/account")).header("Authorization", &format!("Bearer {token}")).call())?;
        if status != 200 {
            return Err(AccountError::Service(format!("the new session didn't work: HTTP {status}")));
        }
        let account: serde_json::Value = serde_json::from_str(&text).map_err(|error| AccountError::Service(error.to_string()))?;
        self.take_details(&account);
        let github_id = account["github_id"].as_u64();
        let saved = Saved {
            token,
            email: account["email"].as_str().map(str::to_owned),
            github_id,
            github_login: github_id.and_then(|id| self.github_login(id)),
        };
        self.keep(Some(saved))?;
        info!(github = github_id.is_some(), "signed in to the VibeBuddy account");
        Ok(())
    }

    /// Revokes this computer's session at the service, then forgets it; forgotten even when the service can't be
    /// reached, since signing out is about this computer. Blocking.
    pub fn sign_out(&self) -> Result<(), AccountError> {
        let token = self.token().ok_or(AccountError::NotSignedIn)?;
        if let Some(base) = &self.base {
            match read(crate::speech::agent(TIMEOUT).post(&format!("{base}/v1/sign-out")).header("Authorization", &format!("Bearer {token}")).send_empty()) {
                Ok((204 | 401, _)) => {}
                Ok((status, _)) => warn!(status, "the service didn't revoke the session; forgetting it anyway"),
                Err(error) => warn!(%error, "the service couldn't be reached to revoke the session; forgetting it anyway"),
            }
        }
        self.keep(None)
    }

    /// Deletes the account at the service, and then forgets the session; an account that can't be deleted stays
    /// signed in, so the person can try again. Blocking.
    pub fn delete(&self) -> Result<(), AccountError> {
        let token = self.token().ok_or(AccountError::NotSignedIn)?;
        let base = self.base.clone().ok_or(AccountError::Unavailable)?;
        let (status, text) = read(crate::speech::agent(TIMEOUT).delete(&format!("{base}/v1/account")).header("Authorization", &format!("Bearer {token}")).call())?;
        // 401: the session is gone, so is the account or at least its hold on this computer.
        if status != 204 && status != 401 {
            return Err(AccountError::Service(format!("the account wasn't deleted: HTTP {status} {text}")));
        }
        info!("deleted the VibeBuddy account");
        self.keep(None)
    }

    /// The service no longer takes the session (a 401): signed out.
    pub fn forget(&self) {
        warn!("the service no longer takes the session; signed out");
        if let Err(error) = self.keep(None) {
            warn!(%error, "couldn't remove the account file");
        }
    }

    fn keep(&self, saved: Option<Saved>) -> Result<(), AccountError> {
        let file = self.file.as_deref().ok_or(AccountError::Unavailable)?;
        let written = match &saved {
            Some(saved) => crate::resident_claude::write_private(file, &serde_json::to_string(saved).expect("serializes")),
            None => match std::fs::remove_file(file) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                other => other,
            },
        };
        written.map_err(|error| AccountError::Service(format!("couldn't write {}: {error}", file.display())))?;
        let signed_in = saved.is_some();
        *self.saved.lock().unwrap() = saved;
        self.signed_in.send_if_modified(|was| std::mem::replace(was, signed_in) != signed_in);
        self.relay_chance.send_modify(|n| *n += 1);
        Ok(())
    }

    /// The GitHub login for a user id, from GitHub's public API; nothing on failure, the App then shows the id.
    fn github_login(&self, id: u64) -> Option<String> {
        let (status, text) = read(crate::speech::agent(Duration::from_secs(5)).get(&format!("{}/user/{id}", self.github_api)).header("Accept", "application/vnd.github+json").call()).ok()?;
        if status != 200 {
            return None;
        }
        serde_json::from_str::<serde_json::Value>(&text).ok()?["login"].as_str().map(str::to_owned)
    }
}

#[cfg(test)]
impl Account {
    /// As if the service had just said the account is on `plan` (`free` or `paid`).
    pub fn read_plan_for_tests(&self, plan: &str) {
        self.take_details(&serde_json::json!({"plan": {"name": plan}}));
    }

    /// As if the service had just said the account is on Free with `upgrade_offered`.
    pub fn read_upgrade_offered_for_tests(&self, offered: bool) {
        self.take_details(&serde_json::json!({"plan": {"name": "free"}, "upgrade_offered": offered}));
    }
}

/// An account signed in with `token`, its file in `dir`, for other modules' tests.
#[cfg(test)]
pub fn signed_in_for_tests(dir: &Path, token: &str) -> Account {
    std::fs::create_dir_all(dir).unwrap();
    crate::resident_claude::write_private(&dir.join(ACCOUNT_FILE), &serde_json::json!({"token": token, "github_id": 1}).to_string()).unwrap();
    Account::new(Some("http://127.0.0.1:9".to_owned()), Some(dir), 7331)
}

fn load(path: &Path) -> Option<Saved> {
    let text = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str(&text) {
        Ok(saved) => Some(saved),
        Err(error) => {
            warn!(%error, path = %path.display(), "the account file can't be read; signed out");
            None
        }
    }
}

fn read(response: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<(u16, String), AccountError> {
    let mut response = response.map_err(|error| AccountError::Service(format!("the service couldn't be reached: {error}")))?;
    let status = response.status().as_u16();
    let text = response.body_mut().read_to_string().unwrap_or_default();
    Ok((status, text))
}

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("the system's random source");
    URL_SAFE_NO_PAD.encode(bytes)
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Enough for a URL query value: everything but unreserved characters is escaped.
fn percent_encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (byte as char).to_string(),
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

/// The page the browser shows after coming back.
pub fn callback_page(result: &Result<(), AccountError>) -> (u16, String) {
    let (status, title, text) = match result {
        Ok(()) => (200, "You're signed in", "You can close this tab and go back to VibeBuddy."),
        Err(AccountError::UnknownSignIn) => (400, "Not signed in", "This sign-in wasn't started from VibeBuddy on this computer, or it took too long. Start again from VibeBuddy's settings."),
        Err(_) => (502, "Not signed in", "VibeBuddy couldn't finish signing in. Try again from VibeBuddy's settings."),
    };
    let html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>{title}</title>\
         <style>body{{font:16px/1.5 system-ui,sans-serif;max-width:24rem;margin:4rem auto;padding:0 1rem}}</style></head>\
         <body><h1>{title}</h1><p>{text}</p></body></html>"
    );
    (status, html)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc;

    /// A fake service: answers each request in turn with the next `(status, body)`, and records what it was sent.
    pub(crate) fn serve(answers: Vec<(u16, &'static str)>) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let record = seen.clone();
        std::thread::spawn(move || {
            for (status, body) in answers {
                let Ok((stream, _)) = listener.accept() else { return };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut head = String::new();
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap_or(0);
                    }
                    head.push_str(&line);
                }
                let mut body_in = vec![0; length];
                reader.read_exact(&mut body_in).unwrap();
                record.lock().unwrap().push(format!("{head}\n{}", String::from_utf8_lossy(&body_in)));
                let mut stream = stream;
                let _ = write!(stream, "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            }
        });
        (url, seen)
    }

    /// A fresh directory of this test's own under the system's temporary directory.
    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vibebuddy-account-{}-{}", std::process::id(), random_token()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn query_value(url: &str, key: &str) -> String {
        let query = url.split_once('?').unwrap().1;
        query.split('&').find_map(|pair| pair.strip_prefix(&format!("{key}="))).unwrap().to_owned()
    }

    fn account(base: &str, dir: &Path) -> Account {
        Account::new(Some(base.to_owned()), Some(dir), 7331)
    }

    #[cfg(unix)]
    #[test]
    fn signs_in_through_the_browser_and_keeps_the_session_privately() {
        let dir = scratch();
        let (service, seen) = serve(vec![(200, r#"{"token":"tok_123"}"#), (200, r#"{"id":"a","email":null,"github_id":60097}"#)]);
        let (github, _) = serve(vec![(200, r#"{"login":"longzhi","id":60097}"#)]);
        let account = account(&service, dir.as_path()).with_github_api(&github);
        let url = account.start().unwrap();
        assert!(url.starts_with(&format!("{service}/v1/sign-in?redirect_uri=http%3A%2F%2F127.0.0.1%3A7331%2Fv1%2Faccount%2Fcallback")));
        assert_eq!(query_value(&url, "code_challenge_method"), "S256");

        account.finish("code-1", &query_value(&url, "state")).unwrap();
        let seen = seen.lock().unwrap();
        // The verifier sent matches the challenge in the URL.
        let body: serde_json::Value = serde_json::from_str(seen[0].split_once("\n\n").unwrap().1).unwrap();
        assert_eq!(body["code"], "code-1");
        assert_eq!(URL_SAFE_NO_PAD.encode(Sha256::digest(body["code_verifier"].as_str().unwrap())), query_value(&url, "code_challenge"));
        assert!(seen[1].to_ascii_lowercase().contains("authorization: bearer tok_123"));

        let status = account.status();
        assert!(status.signed_in && status.available);
        assert_eq!((status.github_id, status.github_login.as_deref()), (Some(60097), Some("longzhi")));
        use std::os::unix::fs::PermissionsExt;
        let file = dir.as_path().join(ACCOUNT_FILE);
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        // A restarted daemon is still signed in.
        assert_eq!(Account::new(Some(service.clone()), Some(dir.as_path()), 7331).token().as_deref(), Some("tok_123"));
    }

    #[test]
    fn refuses_a_callback_it_did_not_start_and_keeps_the_real_one() {
        let dir = scratch();
        let (service, seen) = serve(vec![(200, r#"{"token":"t"}"#), (200, r#"{"email":"a@b.c","github_id":null}"#)]);
        let account = account(&service, dir.as_path());
        assert_eq!(account.finish("code", "anything"), Err(AccountError::UnknownSignIn));
        let url = account.start().unwrap();
        assert_eq!(account.finish("code", "forged-state"), Err(AccountError::UnknownSignIn));
        assert!(seen.lock().unwrap().is_empty());
        account.finish("code", &query_value(&url, "state")).unwrap();
        assert_eq!(account.status().email.as_deref(), Some("a@b.c"));
        // A state works once.
        assert_eq!(account.finish("code", &query_value(&url, "state")), Err(AccountError::UnknownSignIn));
    }

    #[test]
    fn keeps_nothing_when_the_service_refuses_the_code() {
        let dir = scratch();
        let (service, _) = serve(vec![(400, r#"{"error":"invalid_grant"}"#)]);
        let account = account(&service, dir.as_path());
        let url = account.start().unwrap();
        assert!(matches!(account.finish("code", &query_value(&url, "state")), Err(AccountError::Service(_))));
        assert!(!account.status().signed_in);
        assert!(!dir.as_path().join(ACCOUNT_FILE).exists());
    }

    fn signed_in(service: &str, dir: &Path) -> Account {
        crate::resident_claude::write_private(&dir.join(ACCOUNT_FILE), r#"{"token":"tok","github_id":1}"#).unwrap();
        account(service, dir)
    }

    #[test]
    fn signs_out_even_when_the_service_cannot_be_reached() {
        let dir = scratch();
        let account = signed_in("http://127.0.0.1:9", dir.as_path());
        account.sign_out().unwrap();
        assert!(!account.status().signed_in);
        assert!(!dir.as_path().join(ACCOUNT_FILE).exists());
    }

    #[test]
    fn signing_out_revokes_the_session() {
        let dir = scratch();
        let (service, seen) = serve(vec![(204, "")]);
        signed_in(&service, dir.as_path()).sign_out().unwrap();
        let seen = seen.lock().unwrap();
        assert!(seen[0].starts_with("POST /v1/sign-out"));
        assert!(seen[0].to_ascii_lowercase().contains("authorization: bearer tok"));
    }

    #[test]
    fn deletes_the_account_but_stays_signed_in_when_it_fails() {
        let dir = scratch();
        let (service, seen) = serve(vec![(500, "oops"), (204, "")]);
        let account = signed_in(&service, dir.as_path());
        assert!(matches!(account.delete(), Err(AccountError::Service(_))));
        assert!(account.status().signed_in);
        account.delete().unwrap();
        assert!(!account.status().signed_in);
        assert!(seen.lock().unwrap()[1].starts_with("DELETE /v1/account"));
    }

    #[test]
    fn reads_the_plan_and_the_voice_credit_and_shows_none_before_the_first_use() {
        let dir = scratch();
        let (service, seen) = serve(vec![
            (200, r#"{"id":"a","github_id":1,"created_at":"2026-11-01T00:00:00Z","plan":{"name":"free"}}"#),
            (200, r#"{"id":"a","github_id":1,"plan":{"name":"paid","period_end":"2026-12-06T00:00:00.000Z"},"voice_credit_micro_usd":933100}"#),
        ]);
        let account = signed_in(&service, dir.as_path());
        let changes = account.watch_details();
        account.refresh().unwrap();
        assert!(changes.has_changed().unwrap());
        let free = account.status();
        assert_eq!(free.plan, Some(Plan { name: "free".to_owned(), period_end: None }));
        assert_eq!(free.voice_credit_micro_usd, None, "no voice used yet");
        assert!(serde_json::to_value(&free).unwrap().get("voice_credit_micro_usd").is_none(), "absent from the status, not null");
        assert!(seen.lock().unwrap()[0].starts_with("GET /v1/account"));

        account.refresh().unwrap();
        let paid = account.status();
        assert_eq!(paid.plan.as_ref().and_then(|plan| plan.period_end.as_deref()), Some("2026-12-06T00:00:00.000Z"));
        assert_eq!(paid.voice_credit_micro_usd, Some(933_100));
        assert_eq!(serde_json::to_value(&paid).unwrap()["voice_credit_micro_usd"], 933_100);
        account.forget();
        assert_eq!((account.status().plan, account.status().voice_credit_micro_usd), (None, None), "signed out, nothing is shown");
    }

    #[test]
    fn upgrade_is_offered_only_when_the_service_says_so() {
        let dir = scratch();
        let (service, _) = serve(vec![
            (200, r#"{"id":"a","github_id":1,"plan":{"name":"free"},"upgrade_offered":true}"#),
            (200, r#"{"id":"a","github_id":1,"plan":{"name":"free"},"upgrade_offered":false}"#),
            (200, r#"{"id":"a","github_id":1,"plan":{"name":"free"},"upgrade_offered":true}"#),
            // An older service has no such field.
            (200, r#"{"id":"a","github_id":1,"plan":{"name":"free"}}"#),
        ]);
        let account = signed_in(&service, dir.as_path());
        assert!(!account.status().upgrade_offered, "not read yet");
        account.refresh().unwrap();
        assert!(account.status().upgrade_offered);
        assert_eq!(serde_json::to_value(account.status()).unwrap()["upgrade_offered"], true);
        account.refresh().unwrap();
        assert!(!account.status().upgrade_offered);
        account.refresh().unwrap();
        assert!(account.status().upgrade_offered);
        account.refresh().unwrap();
        assert!(!account.status().upgrade_offered, "a missing field means false");
        account.read_upgrade_offered_for_tests(true);
        account.forget();
        assert!(!account.status().upgrade_offered, "signed out");
    }

    #[test]
    fn passes_the_plan_pages_through_with_the_session() {
        let dir = scratch();
        let (service, seen) = serve(vec![
            (200, r#"{"url":"https://checkout.stripe.com/c/pay/cs_test_1"}"#),
            (200, r#"{"url":"https://billing.stripe.com/p/session/test_1"}"#),
        ]);
        let account = signed_in(&service, dir.as_path());
        assert_eq!(account.plan_page(PlanPage::Checkout).as_deref(), Ok("https://checkout.stripe.com/c/pay/cs_test_1"));
        assert_eq!(account.plan_page(PlanPage::Portal).as_deref(), Ok("https://billing.stripe.com/p/session/test_1"));
        let seen = seen.lock().unwrap();
        assert!(seen[0].starts_with("POST /v1/plan/checkout"));
        assert!(seen[1].starts_with("POST /v1/plan/portal"));
        assert!(seen.iter().all(|request| request.to_ascii_lowercase().contains("authorization: bearer tok")));
    }

    #[test]
    fn passes_only_a_stripe_page_for_the_page_asked_for() {
        let dir = scratch();
        let (service, _) = serve(vec![
            (200, r#"{"url":"https://evil.example/c/1"}"#),
            (200, r#"{"url":"http://checkout.stripe.com/c/1"}"#),
            (200, r#"{"url":"https://checkout.stripe.com.evil.example/c/1"}"#),
            (200, r#"{"url":"https://checkout.stripe.com@evil.example/c/1"}"#),
            (200, r#"{"url":"https://billing.stripe.com/p/session/1"}"#),
            (200, r#"{"url":"file:///etc/passwd"}"#),
        ]);
        let account = signed_in(&service, dir.as_path());
        for _ in 0..4 {
            assert!(matches!(account.plan_page(PlanPage::Checkout), Err(AccountError::Service(_))));
        }
        // The portal's host for Checkout is wrong too.
        assert!(matches!(account.plan_page(PlanPage::Checkout), Err(AccountError::Service(_))));
        assert!(matches!(account.plan_page(PlanPage::Portal), Err(AccountError::Service(_))));
        assert!(account.status().signed_in);
    }

    #[test]
    fn a_plan_page_the_service_refuses_says_why() {
        let dir = scratch();
        let (service, _) = serve(vec![
            (409, r#"{"reason":"already_paid"}"#),
            (409, r#"{"reason":"no_subscription"}"#),
            (404, r#"{"error":"not_found"}"#),
            (200, r#"{"no":"url"}"#),
            (401, r#"{"error":"unauthorized"}"#),
        ]);
        let account = signed_in(&service, dir.as_path());
        assert_eq!(account.plan_page(PlanPage::Checkout), Err(AccountError::Conflict("already_paid".to_owned())));
        assert_eq!(account.plan_page(PlanPage::Portal), Err(AccountError::Conflict("no_subscription".to_owned())));
        assert_eq!(account.plan_page(PlanPage::Checkout), Err(AccountError::NotOffered));
        assert!(matches!(account.plan_page(PlanPage::Portal), Err(AccountError::Service(_))));
        // A revoked session signs out, as anywhere else.
        assert_eq!(account.plan_page(PlanPage::Portal), Err(AccountError::NotSignedIn));
        assert!(!account.status().signed_in);
    }

    #[test]
    fn a_plan_page_needs_a_session_and_asks_nothing_without_one() {
        let dir = scratch();
        let (service, seen) = serve(vec![(200, r#"{"url":"https://checkout.stripe.com/c/pay/cs_test_1"}"#)]);
        let account = account(&service, dir.as_path());
        assert_eq!(account.plan_page(PlanPage::Checkout), Err(AccountError::NotSignedIn));
        assert_eq!(account.plan_page(PlanPage::Portal), Err(AccountError::NotSignedIn));
        assert!(seen.lock().unwrap().is_empty());
    }

    #[test]
    fn is_unavailable_without_a_service() {
        let dir = scratch();
        let account = Account::new(None, Some(dir.as_path()), 7331);
        assert_eq!(account.start(), Err(AccountError::Unavailable));
        assert!(!account.status().available);
    }

    #[test]
    fn a_damaged_account_file_means_signed_out() {
        let dir = scratch();
        std::fs::write(dir.as_path().join(ACCOUNT_FILE), "not json").unwrap();
        assert!(!account("http://127.0.0.1:9", dir.as_path()).status().signed_in);
    }
}
