# 日常操作入口。`just` 列出全部；各条背后仍是 tools/ 与 app/scripts/ 里的脚本。

set shell := ["bash", "-euo", "pipefail", "-c"]

default:
    @just --list --unsorted

# Rust 测试加 App 视图模型自检（CI 发版前跑的就是这两样）
test:
    cargo test --workspace
    swift run --package-path app SelfTest

# 固件里不依赖硬件的部分：番茄钟、休闲、语音包
test-firmware:
    tools/test-pomodoro.sh
    tools/test-leisure.sh
    tools/test-voice-pack.sh

# 构建固件三件套到 firmware/build（Release 装包要用）
firmware:
    #!/usr/bin/env bash
    set -euo pipefail
    if ! command -v idf.py >/dev/null; then
        # 激活脚本不能 source（见 LESSONS.md），用 -e 读环境变量。
        while IFS='=' read -r key value; do
            [[ "${key}" == "PATH" ]] && export PATH="${value}:${PATH}"
            [[ "${key}" != "PATH" && "${key}" != "SYSTEM_PATH" ]] && export "${key}=${value}"
        done < <("${HOME}/.espressif/tools/activate_idf_v5.5.3.sh" -e)
        idf() { "${IDF_PYTHON_ENV_PATH}/bin/python" "${IDF_PATH}/tools/idf.py" "$@"; }
    else
        idf() { idf.py "$@"; }
    fi
    idf -C firmware build

# 烧录固件（原生 USB 口；只接 UART 桥时用 tools/flash-bridge.sh）
flash port:
    tools/flash.sh {{port}}

# 装出 app/build/Vibe Buddy.app（要先有固件三件套）
app:
    app/scripts/build-app.sh

# 装包并安装到 /Applications、重启 App。日常用的必须装这里
install:
    app/scripts/build-app.sh --install

# 装包并打成 DMG
dmg: app
    app/scripts/make-dmg.sh "app/build/VibeBuddy-$(git describe --tags --always --dirty)-arm64.dmg"

# 给设备截图：just screenshot /dev/cu.usbmodemXXXX out.png
screenshot port out:
    tools/screenshot.sh {{port}} {{out}}

# 配置 CI 的 Developer ID 签名与公证 secrets（交互向导）
signing-setup:
    tools/setup-release-signing.sh

# 发正式版：just release 0.2.0 → 改 Cargo.toml、提交、打 v0.2.0 标签、推送，CI 接手公证与发布
release version:
    #!/usr/bin/env bash
    set -euo pipefail
    # CI 收到标签才公证并发 Release；标签与 Cargo.toml 不一致时 CI 会拒绝，
    # 所以版本号只在这里改一次，提交与标签同一个 commit。
    v="{{version}}"
    [[ "$v" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "版本号要像 0.2.0，不带 v" >&2; exit 2; }
    [[ "$(git branch --show-current)" == "main" ]] || { echo "只在 main 上发版" >&2; exit 2; }
    [[ -z "$(git status --porcelain)" ]] || { echo "工作树不干净" >&2; exit 2; }
    git fetch -q origin main
    [[ "$(git rev-parse HEAD)" == "$(git rev-parse origin/main)" ]] || { echo "本地 main 与 origin/main 不一致" >&2; exit 2; }
    ! git rev-parse -q --verify "refs/tags/v$v" >/dev/null || { echo "v$v 已存在" >&2; exit 2; }
    perl -pi -e 'BEGIN{$v=shift} s/^version = ".*"/version = "$v"/ && ($done++) unless $done' "$v" Cargo.toml
    cargo update --workspace --offline -q
    git add Cargo.toml Cargo.lock
    git commit -q -m "release: v$v"
    git tag -a "v$v" -m "Vibe Buddy v$v"
    git push -q origin main "v$v"
    echo "已推送 v$v，CI 会公证并发 Release：gh run watch"
