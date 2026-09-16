use std::path::{Path, PathBuf};
use std::process::Command;

/// 把构建时的 git 描述编进二进制。
///
/// 屏幕上要能看出 daemon 是从哪个 commit 来的。语义版本号做不到：两边都写着
/// 0.1.0，而它们可以相差好几天。
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

/// 哪些文件变化时必须重新取版本。
///
/// 只盯 `HEAD` 不够：fast-forward 只改 `refs/heads/<branch>`，`HEAD` 的内容
/// 一个字节都不变，构建脚本不会重跑，二进制里就会留下一个过时的 SHA。
/// 一个会骗人的版本标签比没有标签更糟，所以当前分支的 ref 和 `packed-refs`
/// 也要一起盯。
fn watched_paths() -> Vec<PathBuf> {
    let Some(git_dir) = git_dir() else {
        return Vec::new();
    };
    let head = git_dir.join("HEAD");
    let mut paths = vec![head.clone()];

    // worktree 的 ref 存在主仓库里，由 `commondir` 指出位置。
    let common = match std::fs::read_to_string(git_dir.join("commondir")) {
        Ok(text) => git_dir.join(text.trim()),
        Err(_) => git_dir.clone(),
    };
    if let Ok(text) = std::fs::read_to_string(&head)
        && let Some(reference) = text.strip_prefix("ref:")
    {
        paths.push(common.join(reference.trim()));
        // ref 被打包后松散文件不存在，只有 packed-refs 会变。
        paths.push(common.join("packed-refs"));
    }
    paths
}

/// worktree 的 `.git` 是文件而不是目录，内容指向真正的 git 目录。
fn git_dir() -> Option<PathBuf> {
    let git = Path::new("..").join(".git");
    if git.is_dir() {
        return Some(git);
    }
    let pointer = std::fs::read_to_string(&git).ok()?;
    Some(PathBuf::from(pointer.strip_prefix("gitdir:")?.trim()))
}
