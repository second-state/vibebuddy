use std::process::Command;

/// 链接脚本、Xtensa 链接器检查，以及把构建标识编进固件。
///
/// 页脚与 `DISPLAY READY BUILD` 那一行都显示「git 描述 + 构建时刻」，写法与
/// C 固件（esp_app_desc 的版本 + 每次构建重新生成的时刻）一致：Mac 端要逐字
/// 比对两边的构建标识。
fn main() {
    check_xtensa_linker_available();
    println!("cargo:rustc-link-arg=-Tlinkall.x");

    let describe = run("git", &["describe", "--always", "--tags", "--dirty"]).unwrap_or_else(|| "unknown".to_owned());
    let stamp = run("date", &["+%Y-%m-%d %H:%M"]).unwrap_or_default();
    let describe: String = describe.chars().take(24).collect();
    let stamp: String = stamp.chars().take(16).collect();
    println!("cargo:rustc-env=VIBEBUDDY_FW_BUILD={describe} {stamp}");
    // 不存在的文件：让构建脚本每次都重跑，构建时刻才是这一次的。
    println!("cargo:rerun-if-changed=.build-stamp-never-exists");
}

fn run(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (!text.is_empty()).then_some(text)
}

fn check_xtensa_linker_available() {
    let target = std::env::var("TARGET").unwrap_or_default();
    let Some(chip) = target.strip_prefix("xtensa-").and_then(|rest| rest.strip_suffix("-none-elf")) else {
        return;
    };
    let linker = format!("xtensa-{chip}-elf-gcc");
    if Command::new(&linker).arg("--version").output().is_err() {
        panic!("找不到 Xtensa 链接器 `{linker}`：先 `. ~/export-esp.sh`（没有就先 `espup install`）。");
    }
}
