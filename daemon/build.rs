use std::path::{Path, PathBuf};
use std::process::Command;

/// Bakes the build-time git description into the binary.
///
/// The screen has to show which commit the daemon came from. A semver can't: both sides say
/// 0.1.0, yet they can be days apart.
fn main() {
    for path in watched_paths() {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    println!("cargo:rustc-env=VIBEBUDDY_BUILD={}", describe());
}

fn describe() -> String {
    Command::new("git")
        .args(["describe", "--always", "--tags", "--dirty"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| "unknown".to_owned())
}

/// Which files force the version to be re-read when they change.
///
/// Watching `HEAD` alone isn't enough: a fast-forward only touches `refs/heads/<branch>`, so `HEAD`
/// doesn't change by a single byte, the build script doesn't rerun, and the binary keeps a stale SHA.
/// A version label that lies is worse than none, so the current branch's ref and `packed-refs`
/// are watched too.
fn watched_paths() -> Vec<PathBuf> {
    let Some(git_dir) = git_dir() else {
        return Vec::new();
    };
    let head = git_dir.join("HEAD");
    let mut paths = vec![head.clone()];

    // A worktree's refs live in the main repository, located via `commondir`.
    let common = match std::fs::read_to_string(git_dir.join("commondir")) {
        Ok(text) => git_dir.join(text.trim()),
        Err(_) => git_dir.clone(),
    };
    if let Ok(text) = std::fs::read_to_string(&head)
        && let Some(reference) = text.strip_prefix("ref:")
    {
        paths.push(common.join(reference.trim()));
        // Once refs are packed the loose file is gone; only packed-refs changes.
        paths.push(common.join("packed-refs"));
    }
    paths
}

/// A worktree's `.git` is a file, not a directory, and points at the real git directory.
fn git_dir() -> Option<PathBuf> {
    let git = Path::new("..").join(".git");
    if git.is_dir() {
        return Some(git);
    }
    let pointer = std::fs::read_to_string(&git).ok()?;
    Some(PathBuf::from(pointer.strip_prefix("gitdir:")?.trim()))
}
