#!/usr/bin/env python3
"""Bundle project licenses, exact dependency notices and source directions."""
import json
from pathlib import Path
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]


def run(*args):
    return subprocess.check_output(args, cwd=ROOT, text=True).strip()


def dependency_notices(manifest, target):
    data = json.loads(run(
        "cargo", "about", "generate", "--config", str(ROOT / "packaging/about.toml"),
        "--locked", "--workspace", "--manifest-path", str(ROOT / manifest),
        "--target", target, "--fail", "--format", "json",
    ))
    lines = ["Third-party dependency licenses", "===============================", ""]
    packages = {}
    for license in data["licenses"]:
        lines.append(license["name"])
        for entry in license["used_by"]:
            crate = entry["crate"]
            if not crate["source"]:
                continue
            packages[crate["id"]] = crate
            name, version = crate["name"], crate["version"]
            lines.append(f"  {name} {version}: https://crates.io/api/v1/crates/{name}/{version}/download")
        lines.extend(["", license["text"], ""])
    # NOTICE files can contain attribution beyond the license text itself.
    for crate in sorted(packages.values(), key=lambda c: (c["name"], c["version"])):
        source = Path(crate["manifest_path"]).parent
        if crate["name"] == "libsqlite3-sys":
            sqlite = (source / "sqlite3/sqlite3.c").read_text()
            lines.extend(["Bundled SQLite (public domain): https://www.sqlite.org/copyright.html",
                          sqlite[:sqlite.index("*/") + 2], ""])
        for notice in sorted(source.rglob("*")):
            if notice.is_file() and notice.name.upper().startswith("NOTICE"):
                lines.extend([f"{crate['name']} {crate['version']} — {notice.relative_to(source)}",
                              notice.read_text(), ""])
    return "\n".join(lines)


def main():
    resources = Path(sys.argv[1])
    # The Rust target the app's binaries are built for: the Mac app by default, the Linux package passes its own.
    # `--firmware` writes the notices for the firmware zip instead, which ships on its own (ADR-0010).
    app_target = sys.argv[2] if len(sys.argv) > 2 else "aarch64-apple-darwin"
    firmware = app_target == "--firmware"
    licenses = resources / "licenses"
    licenses.mkdir(parents=True, exist_ok=True)
    for name in ["LICENSE", "LICENSE-ASSETS"]:
        shutil.copyfile(ROOT / name, licenses / name)
    if firmware:
        (licenses / "THIRD-PARTY-FIRMWARE.txt").write_text(
            dependency_notices("firmware-rs/device/Cargo.toml", "xtensa-esp32s3-none-elf"))
        shutil.copyfile(ROOT / "packaging/BOOTLOADER-NOTICES.txt", licenses / "BOOTLOADER-NOTICES.txt")
    else:
        (licenses / "THIRD-PARTY-APP.txt").write_text(dependency_notices("Cargo.toml", app_target))
    revision = run("git", "rev-parse", "HEAD")
    lockfile = "firmware-rs/device/Cargo.lock" if firmware else "Cargo.lock"
    serialport = "" if firmware else ", including the unmodified\nMPL-2.0-covered serialport dependency"
    bootloader_note = (
        "The bootloader's upstream source and build configuration are identified\nin BOOTLOADER-NOTICES.txt."
        if firmware
        else "The firmware is distributed separately and carries its own notices."
    )
    system_note = (
        " The app uses Apple's system frameworks, supplied\nby macOS rather than redistributed in this download."
        if "apple" in app_target
        else ""
    )
    (licenses / "SOURCE.txt").write_text(f"""Vibe Buddy corresponding source
================================

Source revision: {revision}
Source and build/install scripts:
https://github.com/second-state/vibebuddy/tree/{revision}
Source archive:
https://github.com/second-state/vibebuddy/archive/{revision}.tar.gz
Build instructions: CONTRIBUTING.md and README.md at that revision.

Rust dependency versions and checksums are recorded in {lockfile}.
Use cargo fetch --locked with its manifest to retrieve their original
source archives. The THIRD-PARTY file also provides version-specific
source download URLs{serialport}. Cargo's cache contains the complete
crate sources; cargo vendor --locked can copy them for offline builds.

{bootloader_note}{system_note}

The project source is GPL-3.0-or-later. See LICENSE-ASSETS for the scope
of the asset license and excluded third-party audio. Third-party
components retain their own licenses. License texts accompany this download.
""")
    print("Bundled project licenses, third-party notices and corresponding-source directions")


if __name__ == "__main__":
    main()
