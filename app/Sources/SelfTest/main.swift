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
 "device":{"connected":true,"port":"/dev/cu.usbmodem1","bridge":true,"mode":"pomodoro","firmware_build":"abc1234-dirty 2026-09-16 13:11","voice":"xiaohe2","volume":65},
 "today":{"done":3,"asks":1,"busy_seconds":4980},
 "hooks":{"codex":"2026-09-16T13:31:30.060465+08:00","claude":null},
 "operation":{"kind":"voice_pack","state":"running","progress":0.42,"message":"writing hsiaoyu"},
 "config":{"voice":"xiaohe2","notify_link":true}}
"""
do {
    let status = try StatusCoding.decoder().decode(Status.self, from: Data(sample.utf8))
    check(status.device.mode == "pomodoro", "mode decodes")
    check(status.device.volume == 65, "volume decodes")
    check(status.hooks.codex != nil && status.hooks.claude == nil, "hook timestamps decode")
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
    check(Firmware.updateAvailable(device: status.device.firmwareBuild, bundled: "def5678 2026-09-17 09:00"), "different hash offers an update")
    check(!Firmware.updateAvailable(device: status.device.firmwareBuild, bundled: "abc1234-dirty 2026-09-16 13:11"), "same hash offers no update")
    check(!Firmware.updateAvailable(device: nil, bundled: "def5678 x"), "no nagging when the box reports no build")
    check(Firmware.foreign(connected: true, device: nil, connectedFor: 6), "connected but silent past the grace period: factory firmware")
    check(!Firmware.foreign(connected: true, device: nil, connectedFor: 1), "just connected, still within the grace period: not foreign")
    check(!Firmware.foreign(connected: true, device: "abc 1", connectedFor: 60), "a reported build means it is ours")
    check(!Firmware.foreign(connected: false, device: nil, connectedFor: 60), "not connected: not foreign")
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

// 4. Voice catalog: every entry has a language, and the picker puts the UI language first.
let catalog = VoiceCatalogEntry.all
check(Set(catalog.map(\.id)).count == catalog.count, "voice ids are unique")
check(catalog.contains { $0.language == .en } && catalog.contains { $0.language == .zh }, "catalog has voices in both languages")
let englishFirst = VoiceCatalogEntry.sorted(catalog, preferring: .en)
check(englishFirst.first?.language == .en && englishFirst.count == catalog.count, "English UI lists English voices first")
check(VoiceCatalogEntry.sorted(catalog, preferring: .zh).first?.id == "wanwanxiaohe", "Chinese UI keeps catalog order")

if failures > 0 {
    print("\(failures) failure(s)")
    exit(1)
}
print("self-test passed")
