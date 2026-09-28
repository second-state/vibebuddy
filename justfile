# Entry point for everyday tasks. `just` lists them all; each one still runs the scripts in tools/ and app/scripts/.

set shell := ["bash", "-euo", "pipefail", "-c"]

default:
    @just --list --unsorted

# Rust tests plus the app view-model self-test (the same two things CI runs before a release)
test:
    cargo test --workspace
    swift run --package-path app SelfTest

# Tests for the hardware-independent firmware (firmware-core), then a pixel-by-pixel comparison of the Rust and C firmware screens
test-firmware:
    cargo test -p vibebuddy-firmware-core
    tools/compare-display.sh

# Build the Rust firmware images into firmware-rs/device/build (needed for a Release app bundle); FAST_CLOCK=1 builds the acceptance-test variant
firmware:
    tools/build-firmware.sh

# Flash the Rust firmware (native USB port; with only the UART bridge, use tools/flash-bridge.sh)
flash port:
    tools/flash.sh {{port}}

# Fall back to the C firmware: build it with ESP-IDF and flash it (if the Rust firmware misbehaves)
flash-c port:
    tools/flash-c.sh {{port}}

# The C firmware's own host tests: pomodoro, leisure, voice pack
test-firmware-c:
    tools/test-pomodoro.sh
    tools/test-leisure.sh
    tools/test-voice-pack.sh

# Build app/build/Vibe Buddy.app (needs the three firmware images first)
app:
    app/scripts/build-app.sh

# Build, install to /Applications and restart the app. The copy you use day to day must live there
install:
    app/scripts/build-app.sh --install

# Build the app and package it as a DMG
dmg: app
    app/scripts/make-dmg.sh "app/build/VibeBuddy-$(git describe --tags --always --dirty)-arm64.dmg"

# Screenshot the device: just screenshot /dev/cu.usbmodemXXXX out.png
screenshot port out:
    tools/screenshot.sh {{port}} {{out}}

# Configure the Developer ID signing and notarization secrets for CI (interactive wizard)
signing-setup:
    tools/setup-release-signing.sh

# Cut a release: just release 0.2.0 → bump Cargo.toml, commit, tag v0.2.0, push; CI takes over notarization and publishing
release version:
    #!/usr/bin/env bash
    set -euo pipefail
    # CI only notarizes and publishes a Release when it gets a tag, and rejects a tag that disagrees with Cargo.toml,
    # so the version is changed only here, once, with the commit and the tag on the same commit.
    v="{{version}}"
    [[ "${v}" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "version must look like 0.2.0, without a leading v" >&2; exit 2; }
    [[ "$(git branch --show-current)" == "main" ]] || { echo "releases are cut from main only" >&2; exit 2; }
    [[ -z "$(git status --porcelain)" ]] || { echo "working tree is not clean" >&2; exit 2; }
    git fetch -q origin main
    [[ "$(git rev-parse HEAD)" == "$(git rev-parse origin/main)" ]] || { echo "local main differs from origin/main" >&2; exit 2; }
    ! git rev-parse -q --verify "refs/tags/v${v}" >/dev/null || { echo "v${v} already exists" >&2; exit 2; }
    # Always write ${v}, not $v: a character right after $v can be read as part of the name
    # (full-width punctuation in the old Chinese messages did exactly that).
    perl -pi -e 'BEGIN{$new=shift} s/^version = ".*"/version = "$new"/ && ($done++) unless $done' "${v}" Cargo.toml
    cargo update --workspace --offline -q
    git add Cargo.toml Cargo.lock
    git commit -q -m "release: v${v}"
    git tag -a "v${v}" -m "Vibe Buddy v${v}"
    git push -q origin main "v${v}"
    echo "pushed v${v}; CI will notarize and publish the Release: gh run watch"
