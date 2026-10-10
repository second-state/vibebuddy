// Self-test for the view-model seams: status JSON in; menu copy, icon state and hook config merges out.
// `swift run SelfTest`; exits non-zero if any check fails.
import Foundation
import VibeBuddyCore

var failures = 0
func check(_ condition: Bool, _ message: String, file: String = #file, line: Int = #line) {
    if !condition {
        failures += 1
        print("FAIL \(line): \(message)")
    }
}

// 1. Status JSON decoding, with microsecond timestamps and time zones.
let sample = """
{"daemon":{"build":"0.3.0 abc1234 2026-09-16 10:50","app_version":"0.3.0"},
 "device":{"connected":true,"port":"/dev/cu.usbmodem1","bridge":true,"mode":"pomodoro","firmware_build":"abc1234-dirty 2026-09-16 13:11","firmware_version":"0.2.2","voice":"xiaohe2","volume":65},
 "today":{"done":3,"asks":1,"busy_seconds":4980},
 "hooks":{"codex":"2026-09-16T13:31:30.060465+08:00","claude":null},
 "operation":{"kind":"voice_pack","state":"running","progress":0.42,"message":"writing hsiaoyu"},
 "config":{"voice":"xiaohe2","notify_link":true,"check_updates":null},
 "updates":{"enabled":true,"last_check":"2026-10-07T12:43:11.068011+08:00","error":null,
  "app":{"version":"0.4.0","url":"https://example/app.dmg","notes":{"en":"- New"}},"unsupported_app":false,
  "firmware":{"version":"0.4.0","notes":{"en":"- Fix","zh-Hans":"- 修复"},"directory":"/tmp/fw/0.4.0","newer_than_box":true}}}
"""
do {
    let status = try StatusCoding.decoder().decode(Status.self, from: Data(sample.utf8))
    check(status.device.mode == "pomodoro", "mode decodes")
    check(status.device.volume == 65, "volume decodes")
    check(status.device.firmwareVersion == "0.2.2", "firmware version decodes")
    check(Firmware.label(version: "0.2.2", build: "abc 1") == "0.2.2 · abc 1", "version leads the firmware label")
    check(Firmware.label(version: nil, build: "abc 1") == "abc 1", "firmware without a version shows its build")
    check(status.hooks.codex != nil && status.hooks.claude == nil, "hook timestamps decode")
    check(status.hooks.opencode == nil && status.hooks.lastEvent(.codex) == status.hooks.codex, "a daemon without OpenCode and Copilot still decodes")
    check(status.operation?.kind == .voicePack && status.operation?.progress == 0.42, "operation decodes")
    let menu = MenuState.derive(status: status, daemonAlive: true)
    check(menu.icon == .online, "online icon")
    check(menu.deviceLine == "Box online · firmware abc1234-dirty", "device line: \(menu.deviceLine)")
    check(menu.modeLine == "Mode: Pomodoro", "mode line: \(menu.modeLine)")
    check(menu.todayLine == "Today: done 3 · asks 1 · busy 1 h 23 min", "today line: \(menu.todayLine)")
    var offline = status
    offline.device.connected = false
    let offlineMenu = MenuState.derive(status: offline, daemonAlive: true)
    check(offlineMenu.icon == .offline && offlineMenu.deviceLine == "Box not found", "offline menu")
    let down = MenuState.derive(status: nil, daemonAlive: false)
    check(down.icon == .daemonDown && down.deviceLineIsAction, "daemon-down menu")
    check(status.config.checkUpdates == nil, "check_updates null follows the build")
    check(status.updates?.app?.version == "0.4.0" && status.updates?.lastCheck != nil, "updates decode")
    check(Firmware.updateAvailable(status.updates), "a downloaded firmware newer than the box is offered")
    check(Firmware.files(of: status.updates?.firmware)?.app.path == "/tmp/fw/0.4.0/vibebuddy-fw.bin", "flashing uses the downloaded images")
    var notDownloaded = status.updates
    notDownloaded?.firmware?.directory = nil
    check(!Firmware.updateAvailable(notDownloaded), "nothing to flash before the download")
    var sameAsBox = status.updates
    sameAsBox?.firmware?.newerThanBox = false
    check(!Firmware.updateAvailable(sameAsBox), "the box already runs it")
    check(!Firmware.updateAvailable(nil), "an older daemon offers nothing")
    check(Firmware.notes(["en": "a", "zh-Hans": "b"], chinese: true) == "b" && Firmware.notes(["en": "a"], chinese: true) == "a", "notes fall back to English")
    check(!Firmware.foreign(status.device), "an older daemon without the field: not foreign")
    check(Firmware.foreign(DeviceState(connected: true, foreignFirmware: true)), "the daemon judged it other firmware")
    check(!Firmware.foreign(DeviceState(connected: true)), "just connected, not judged yet: not foreign")
    check(!Firmware.foreign(DeviceState(connected: true, firmwareBuild: "abc 1", foreignFirmware: true)), "a reported build means it is ours")
    check(!Firmware.foreign(DeviceState(connected: false, foreignFirmware: true)), "not connected: not foreign")
    check(status.device.pin == nil && status.device.candidates == nil, "an older daemon: no pin, no candidates")
    let choosing = try StatusCoding.decoder().decode(DeviceState.self, from: Data("""
    {"connected":false,"bridge":false,"pin":{"variable":"VIBEBUDDY_SERIAL_PORT","value":"/dev/cu.usbmodem8401"},
     "candidates":[{"port":"/dev/cu.usbmodem1101","usb_serial":"30:ED:A0:A4:0D:08"},{"port":"/dev/cu.usbserial-840","usb_serial":null}]}
    """.utf8))
    check(choosing.pin == SerialPin(variable: "VIBEBUDDY_SERIAL_PORT", value: "/dev/cu.usbmodem8401"), "pin decodes")
    check(choosing.candidates?.map(\.usbSerial) == ["30:ED:A0:A4:0D:08", nil], "candidates decode, with or without a serial")
    check(status.device.pairedComputers == nil && status.device.boxKey == nil, "an older daemon: no pairing")
    let paired = try StatusCoding.decoder().decode(DeviceState.self, from: Data("""
    {"connected":true,"bridge":false,"box_key":"BOX","computer_key":"MINE",
     "paired_computers":[{"key":"MINE","name":"dragon's MacBook"},{"key":"OTHER","name":"omarchy"}]}
    """.utf8))
    check(paired.boxKey == "BOX" && paired.computerKey == "MINE", "pairing keys decode")
    check(paired.pairedComputers?.map(\.name) == ["dragon's MacBook", "omarchy"], "paired computers decode")
    let wifi = try StatusCoding.decoder().decode(DeviceState.self, from: Data("""
    {"connected":true,"bridge":false,"network":true,"port":"192.168.1.23:7340","wifi_network":"Home","wifi_address":"192.168.1.23"}
    """.utf8))
    check(wifi.network == true && wifi.wifiNetwork == "Home" && wifi.wifiAddress == "192.168.1.23", "Wi-Fi fields decode")
} catch {
    check(false, "status decoding threw: \(error)")
}

// 1b. Firmware package: found even inside an extra zip folder; magic numbers checked; build.txt wins over the image's version string.
do {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent("selftest-fw-\(UUID().uuidString)")
    let nested = root.appendingPathComponent("firmware")
    try FileManager.default.createDirectory(at: nested, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    func write(_ name: String, _ bytes: [UInt8]) throws { try Data(bytes).write(to: nested.appendingPathComponent(name)) }
    var app = [UInt8](repeating: 0, count: 0x60)
    app[0] = 0xE9
    app[0x20...0x23] = [0x32, 0x54, 0xCD, 0xAB]
    for (i, c) in "v9.9.9-dirty".utf8.enumerated() { app[0x30 + i] = c }
    try write(FirmwarePackage.bootloaderName, [0xE9, 0x03, 0x02, 0x4F])
    try write(FirmwarePackage.partitionTableName, [0xAA, 0x50, 0x01, 0x02])
    try write(FirmwarePackage.appName, app)
    let bare = try FirmwarePackage.inspect(directory: root)
    check(bare.build == "v9.9.9-dirty", "without build.txt the image version is used: \(bare.build)")
    try Data("abc1234 2026-09-22 10:00\n".utf8).write(to: nested.appendingPathComponent(FirmwarePackage.buildName))
    let stamped = try FirmwarePackage.inspect(directory: root)
    check(stamped.build == "abc1234 2026-09-22 10:00", "build.txt wins: \(stamped.build)")
    check(stamped.version == nil, "without version.txt there is no version")
    try Data("0.3.0\n".utf8).write(to: nested.appendingPathComponent(FirmwarePackage.versionName))
    let versioned = try FirmwarePackage.inspect(directory: root)
    check(versioned.version == "0.3.0", "version.txt is read: \(versioned.version ?? "nil")")
    check(stamped.app.lastPathComponent == FirmwarePackage.appName, "finds the app image in a subdirectory")
    try write(FirmwarePackage.partitionTableName, [0x00, 0x00])
    do {
        _ = try FirmwarePackage.inspect(directory: root)
        check(false, "a bad partition table is rejected")
    } catch let failure as FirmwarePackage.Failure {
        check(failure == .notAnImage(FirmwarePackage.partitionTableName), "a bad partition table names itself: \(failure)")
    }
    try FileManager.default.removeItem(at: nested.appendingPathComponent(FirmwarePackage.appName))
    do {
        _ = try FirmwarePackage.inspect(directory: root)
        check(false, "a missing app image is rejected")
    } catch let failure as FirmwarePackage.Failure {
        check(failure == .missing(FirmwarePackage.appName), "a missing app image reports the missing file: \(failure)")
    }
} catch {
    check(false, "status decoding threw: \(error)")
}

// 2. Hook config merge: others' hooks untouched, old Python entries replaced, clean after removal.
let existing: [String: Any] = [
    "hooks": [
        "PreToolUse": [["matcher": "Bash", "hooks": [["type": "command", "command": "'/Users/x/.codex/hooks/rtk-rewrite.sh'"]]]],
        "Stop": [["hooks": [["type": "command", "command": "/usr/bin/python3 /old/codex-hook.py", "timeout": 2]]]],
    ],
    "model": "gpt-5",
]
let binary = "/Users/x/Library/Application Support/VibeBuddy/bin/vibebuddy-hook"
let installed = HookConfig.install(into: existing, agent: .codex, binary: binary)
check(HookConfig.isInstalled(in: installed, agent: .codex, binary: binary), "detected as installed after install")
check(!HookConfig.isInstalled(in: existing, agent: .codex, binary: binary), "detected as not installed before install")
check((installed["model"] as? String) == "gpt-5", "other keys are kept")
let installedHooks = installed["hooks"] as! [String: Any]
let preToolUse = installedHooks["PreToolUse"] as! [[String: Any]]
check(preToolUse.count == 1 && (preToolUse[0]["matcher"] as? String) == "Bash", "someone else's PreToolUse is untouched (it is not in the Codex event list)")
let stop = installedHooks["Stop"] as! [[String: Any]]
let stopCommands = stop.flatMap { ($0["hooks"] as! [[String: Any]]).map { $0["command"] as! String } }
check(stopCommands == ["\"\(binary)\" codex"], "the old Python entry is replaced: \(stopCommands)")
check(HookAgent.codex.events.allSatisfy { installedHooks[$0] != nil }, "all six events are present")
let removed = HookConfig.removed(from: installed)
let removedHooks = removed["hooks"] as! [String: Any]
check(removedHooks.keys.sorted() == ["PreToolUse"], "after removal only other hooks remain: \(removedHooks.keys.sorted())")
let diff = HookConfig.describeChange(from: existing, to: installed, agent: .codex)
check(diff.contains("- Stop: /usr/bin/python3 /old/codex-hook.py") && diff.contains("+ Stop: \"\(binary)\" codex"), "change description: \(diff)")
check(HookAgent.claude.events.count == 8, "Claude has eight events")
check(HookAgent.opencode.ownsFile && HookAgent.copilot.ownsFile && HookAgent.pi.ownsFile && !HookAgent.codex.ownsFile, "OpenCode, Copilot and Pi get a file of their own")
check(HookConfig.describeOwnedFile(from: nil, to: "a\nb\n") == ["+ a", "+ b"], "writing an owned file shows every line")
check(HookConfig.describeOwnedFile(from: "a\n", to: nil) == ["- a"], "removing an owned file shows what goes")
check(HookConfig.describeOwnedFile(from: "a\n", to: "a\n").isEmpty, "an unchanged owned file needs no write")
// Codex trust hint: warn only if the config is newer than the last event; no verdict without events; no warning if the file time is unreadable.
let earlier = Date(timeIntervalSince1970: 1_000)
let later = Date(timeIntervalSince1970: 2_000)
check(HookConfig.codexTrustHint(configModifiedAt: later, lastEvent: nil) == .waitingFirstEvent, "no event yet: waiting")
check(HookConfig.codexTrustHint(configModifiedAt: later, lastEvent: earlier) == .changedSinceLastEvent(later), "config newer than the last event: needs re-trust")
check(HookConfig.codexTrustHint(configModifiedAt: earlier, lastEvent: later) == .trusted, "event received after the change: running")
check(HookConfig.codexTrustHint(configModifiedAt: nil, lastEvent: later) == .trusted, "no alarm when the file time is unreadable")

// 3. Voice pack parsing.
var pack = Data(count: 256)
pack.replaceSubrange(0..<4, with: Data("VBVP".utf8))
pack.replaceSubrange(16..<23, with: Data("hsiaoyu".utf8))
var offset = 256
for index in 0..<5 {
    let length = 10 * (index + 1)
    for (i, byte) in withUnsafeBytes(of: UInt32(offset).littleEndian, Array.init).enumerated() { pack[48 + index * 4 + i] = byte }
    for (i, byte) in withUnsafeBytes(of: UInt32(length).littleEndian, Array.init).enumerated() { pack[68 + index * 4 + i] = byte }
    offset += length
}
pack.append(Data(repeating: 1, count: 150))
if let parsed = VoicePack(data: pack) {
    check(parsed.voiceID == "hsiaoyu", "voice id")
    check(parsed.clips[4] == (256 + 100)..<(256 + 150), "fifth clip range")
    check(parsed.previewPCM().count == 150 + 4 * 28_800, "preview length \(parsed.previewPCM().count)")
} else {
    check(false, "voice pack parses")
}
check(VoicePack(data: Data("garbage".utf8)) == nil, "garbage is not a voice pack")

// A Character pack: a 1024-byte header; needs input has one line of 4 samples, done one of 3.
var character = Data(count: 1024)
character.replaceSubrange(0..<4, with: Data("VBCP".utf8))
character.replaceSubrange(16..<22, with: Data("sample".utf8))
character[52] = 1
character[53] = 12
character[54] = 2
character[58] = 1 // needs input: first line 0, one line
character[60] = 1; character[62] = 1 // done: first line 1, one line
for (line, (offset, samples)) in [(1024, 4), (1026, 3)].enumerated() {
    for (i, byte) in withUnsafeBytes(of: UInt32(offset).littleEndian, Array.init).enumerated() { character[128 + line * 8 + i] = byte }
    for (i, byte) in withUnsafeBytes(of: UInt32(samples).littleEndian, Array.init).enumerated() { character[132 + line * 8 + i] = byte }
}
character.append(Data([0x77, 0x77, 0x07, 0x00]))
if let parsed = VoicePack(data: character) {
    check(parsed.voiceID == "sample" && parsed.isCharacter, "character id")
    check(parsed.previewLines.count == 2, "a preview line per ordinary occasion with a pool")
    // 4 and 3 samples at 16 kHz become 6 and 5 stereo frames at 24 kHz, with a 300 ms gap between.
    check(parsed.previewPCM().count == (6 + 5) * 4 + 28_800, "character preview length \(parsed.previewPCM().count)")
} else {
    check(false, "character pack parses")
}

// A look from a drawing: a white background with an orange figure (a 20 × 60 block) and white eyes inside.
var drawing = [UInt8](repeating: 255, count: 100 * 100 * 4)
for y in 20..<80 {
    for x in 40..<60 {
        let at = (y * 100 + x) * 4
        let eye = y == 30 && (x == 45 || x == 54)
        drawing[at] = eye ? 255 : 250; drawing[at + 1] = eye ? 255 : 120; drawing[at + 2] = eye ? 255 : 20
    }
}
if let look = try? LookBuilder.build([RGBAImage(width: 100, height: 100, pixels: drawing)]) {
    check(look.count == 8 + 32 + 4 * 1536 && look.prefix(4) == Data("LOOK".utf8), "look size and magic")
    // The figure is 60 tall, the cell 64: rows 4 to 63, bottom-aligned; the top rows stay transparent.
    check(look[40] == 0 && look[40 + 4 * 24 + 12] != 0, "a transparent top, the figure from row 4")
    // White eyes inside the figure survive the background removal: some index other than the orange.
    let frame = look[40..<(40 + 1536)]
    check(Set(frame.flatMap { [$0 & 15, $0 >> 4] }).count >= 3, "transparent, orange and white")
    if let base = VoicePack(data: character), let worn = base.withLook(look, id: "mine"), let parsed = VoicePack(data: worn) {
        check(parsed.voiceID == "mine" && worn[4] == 2, "a version 2 pack under the new id")
        let offset = VoicePack.u32(worn, 1008), length = VoicePack.u32(worn, 1012)
        check(offset % 4 == 0 && worn.subdata(in: offset..<(offset + length)) == look, "the look sits word-aligned after the lines")
        check(VoicePack.u32(worn, 1020) == Int(CRC32.of(worn.prefix(1020))), "header CRC")
        check(VoicePack.u32(worn, 12) == Int(CRC32.of(worn.suffix(from: 1024))), "payload CRC")
        check(parsed.previewLines.count == base.previewLines.count, "the lines are borrowed as they were")
        check(parsed.look == look, "the pack hands its look back")
        let frames = LookBuilder.frames(of: look)
        check(frames?.count == 4 && frames?[0].pixel(0, 0).3 == 0 && frames?[0].pixel(24, 40).3 == 255, "a look decodes to four frames")
        check(base.withLook(look, id: String(repeating: "x", count: 32)) == nil, "an id too long for the header")
    } else {
        check(false, "a Character pack takes a look")
    }
} else {
    check(false, "a look builds from one drawing")
}
check(CRC32.of(Data("123456789".utf8)) == 0xCBF4_3926, "CRC32 as in zlib")

// The app lays a pack out byte for byte as tools/character_pack.py does: rebuilding a shipped pack
// from its own pools gives it back unchanged.
for id in ["ada", "wanwanxiaohe"] {
    if let data = try? Data(contentsOf: URL(fileURLWithPath: "characters/\(id)/pack.bin")), let pack = VoicePack(data: data), let pools = pack.pools {
        check(VoicePack.build(id: pack.voiceID, pools: pools, look: pack.look) == data, "\(id): rebuilt byte for byte")
    }
}

// A form of address: the variant's lines replace the first lines of each pool, the rest stay.
if let base = VoicePack(data: character), let pools = base.pools {
    check(pools.count == 12 && pools[0].count == 1 && pools[1].count == 1 && pools[1][0].samples == 3, "pools read back")
    let variantPools: [[(audio: Data, samples: Int)]] = [[(Data([0x11, 0x22, 0x33]), 6)], [], []]
    if let variantData = VoicePack.build(id: "sample", pools: variantPools, look: nil), let variant = VoicePack(data: variantData),
       let addressed = base.withAddress(variant).flatMap(VoicePack.init(data:)), let after = addressed.pools {
        check(after[0][0].audio == Data([0x11, 0x22, 0x33]) && after[0][0].samples == 6, "the needs-input line is the variant's")
        check(after[1][0].audio == pools[1][0].audio, "the done line is untouched")
        check(addressed.voiceID == "sample" && addressed.look == base.look, "same id, same look")
        check(VoicePack.u32(addressed.data, 12) == Int(CRC32.of(addressed.data.suffix(from: 1024))), "payload CRC after recomposing")
    } else {
        check(false, "a pack takes a form of address")
    }
} else {
    check(false, "pools of a Character pack")
}

// 4. Voice catalog: every entry has a language, and the picker puts the UI language first.
let catalog = VoiceCatalogEntry.all
check(Set(catalog.map(\.id)).count == catalog.count, "voice ids are unique")
check(catalog.contains { $0.language == .en } && catalog.contains { $0.language == .zh }, "catalog has voices in both languages")
let englishFirst = VoiceCatalogEntry.sorted(catalog, preferring: .en)
check(englishFirst.first?.language == .en && englishFirst.count == catalog.count, "English UI lists English voices first")
check(VoiceCatalogEntry.sorted(catalog, preferring: .zh).first?.id == "wanwanxiaohe", "Chinese UI keeps catalog order")

// 5. Switching the UI language suggests a voice only when the box speaks the other language.
check(VoiceCatalogEntry.language(ofVoice: "builtin") == .en, "the built-in voice is English")
check(VoiceCatalogEntry.language(ofVoice: "hsiaoyu") == nil, "an unknown voice has no language")
check(VoiceCatalogEntry.switchSuggestion(boxVoice: "builtin", to: .zh, bundled: catalog)?.id == "wanwanxiaohe", "English box, Chinese UI: suggest the first Chinese voice")
check(VoiceCatalogEntry.switchSuggestion(boxVoice: "wanwanxiaohe", to: .en, bundled: catalog)?.id == "ada", "Chinese box, English UI: suggest Ada")
check(VoiceCatalogEntry.switchSuggestion(boxVoice: "ada", to: .en, bundled: catalog) == nil, "the box already speaks the UI language")
check(VoiceCatalogEntry.switchSuggestion(boxVoice: "hsiaoyu", to: .zh, bundled: catalog) == nil, "unknown box voice: don't guess")
check(VoiceCatalogEntry.switchSuggestion(boxVoice: "builtin", to: .zh, bundled: catalog.filter { $0.language == .en }) == nil, "no Chinese pack bundled: nothing to offer")
check(VoiceCatalogEntry.switchSuggestion(boxVoice: "wanwanxiaohe", to: .en, bundled: catalog.filter { $0.id == "luna" })?.id == "luna", "skip Characters this build doesn't ship")

// "Report a problem" opens a new issue with the versions in its body, "+" and line breaks intact.
let report = IssueReport.url(summary: "App 0.3.5 (12)\nmacOS 15.1+beta")
let reportBody = URLComponents(url: report, resolvingAgainstBaseURL: false)?.queryItems?.first { $0.name == "body" }?.value
check(report.absoluteString.hasPrefix("https://github.com/second-state/vibebuddy/issues/new?body="), "report: a new issue on GitHub")
check(reportBody?.contains("App 0.3.5 (12)\nmacOS 15.1+beta") == true && !report.absoluteString.contains("+"), "report: the versions survive the URL")

// The bundle's file name moved from "Vibe Buddy.app" to "VibeBuddy.app"; only installed copies follow.
let applications = URL(fileURLWithPath: "/Applications")
let oldBundle = applications.appendingPathComponent(BundleName.old)
let newBundle = applications.appendingPathComponent(BundleName.current)
check(BundleName.step(for: oldBundle, exists: { _ in false }) == .rename(to: newBundle), "bundle: an old copy alone is renamed")
check(BundleName.step(for: oldBundle, exists: { $0 == newBundle }) == .replaceWith(newBundle), "bundle: an old copy gives way to a new one")
check(BundleName.step(for: newBundle, exists: { $0 == oldBundle }) == .removeOld(oldBundle), "bundle: a new copy removes the old one")
check(BundleName.step(for: newBundle, exists: { _ in false }) == .none, "bundle: a new copy alone stays")
let devBuild = URL(fileURLWithPath: "/Users/me/vibe-buddy/app/build").appendingPathComponent(BundleName.old)
check(BundleName.step(for: devBuild, exists: { _ in false }) == .none, "bundle: a build outside Applications is left alone")

if failures > 0 {
    print("\(failures) failure(s)")
    exit(1)
}
print("self-test passed")
