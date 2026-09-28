use std::process::Command;

/// Linker script, Xtensa linker check, and compiling the build stamp into the firmware.
///
/// The footer and the `DISPLAY READY BUILD` line both show "git description + build time", written
/// the same way as the C firmware (esp_app_desc's version + a time regenerated on every build): the
/// Mac compares the two sides' build stamps byte for byte.
fn main() {
    check_xtensa_linker_available();
    println!("cargo:rustc-link-arg=-Tlinkall.x");

    let describe = run("git", &["describe", "--always", "--tags", "--dirty"]).unwrap_or_else(|| "unknown".to_owned());
    let stamp = run("date", &["+%Y-%m-%d %H:%M"]).unwrap_or_default();
    let describe: String = describe.chars().take(24).collect();
    let stamp: String = stamp.chars().take(16).collect();
    println!("cargo:rustc-env=VIBEBUDDY_FW_BUILD={describe} {stamp}");
    // A file that never exists: rerun the build script every time so the build time is this build's.
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
        panic!("Xtensa linker `{linker}` not found: run `. ~/export-esp.sh` first (or `espup install` if you don't have it).");
    }
}
