use std::path::{Path, PathBuf};
use std::process::Command;

/// 把构建时的 git 描述编进二进制。
///
/// 屏幕上要能一眼看出固件和 daemon 是不是同一份代码。语义版本号做不到这件
/// 事：两边都写着 0.1.0，而它们可以相差好几天。
fn main() {
    // HEAD 移动后必须重新取，否则显示的是上一个 commit。
    if let Some(head) = head_path() {
        println!("cargo:rerun-if-changed={}", head.display());
    }
    println!("cargo:rustc-env=BEACON_BUILD={}", describe());
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

/// worktree 的 `.git` 是文件而不是目录，HEAD 在它指向的地方。
fn head_path() -> Option<PathBuf> {
    let git = Path::new("..").join(".git");
    if git.is_dir() {
        return Some(git.join("HEAD"));
    }
    let pointer = std::fs::read_to_string(&git).ok()?;
    Some(Path::new(pointer.strip_prefix("gitdir:")?.trim()).join("HEAD"))
}
