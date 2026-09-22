// 视图模型缝的自检：一段状态 JSON 进，菜单文案、图标状态、Hook 配置合并结果出。
// `swift run SelfTest`；任何一条不过就以非零退出。
import Foundation
import VibeBuddyCore

var failures = 0
func check(_ condition: Bool, _ message: String, file: String = #file, line: Int = #line) {
    if !condition {
        failures += 1
        print("失败 \(line): \(message)")
    }
}

// 1. 状态 JSON 解码，时间带微秒与时区。
let sample = """
{"daemon":{"build":"0.3.0 abc1234 2026-09-16 10:50","app_version":"0.3.0"},
 "device":{"connected":true,"port":"/dev/cu.usbmodem1","bridge":true,"mode":"pomodoro","firmware_build":"abc1234-dirty 2026-09-16 13:11","voice":"xiaohe2","volume":65},
 "today":{"done":3,"asks":1,"busy_seconds":4980},
 "hooks":{"codex":"2026-09-16T13:31:30.060465+08:00","claude":null},
 "operation":{"kind":"voice_pack","state":"running","progress":0.42,"message":"正在写入 hsiaoyu"},
 "config":{"voice":"xiaohe2","notify_link":true}}
"""
do {
    let status = try StatusCoding.decoder().decode(Status.self, from: Data(sample.utf8))
    check(status.device.mode == "pomodoro", "模式解码")
    check(status.device.volume == 65, "音量解码")
    check(status.hooks.codex != nil && status.hooks.claude == nil, "Hook 时间解码")
    check(status.operation?.kind == .voicePack && status.operation?.progress == 0.42, "操作解码")
    let menu = MenuState.derive(status: status, daemonAlive: true)
    check(menu.icon == .online, "在线图标")
    check(menu.deviceLine == "盒子在线 · 固件 abc1234-dirty", "设备行：\(menu.deviceLine)")
    check(menu.modeLine == "模式：番茄钟", "模式行：\(menu.modeLine)")
    check(menu.todayLine == "今天：完成 3 · 确认 1 · 忙碌 1 小时 23 分", "战绩行：\(menu.todayLine)")
    var offline = status
    offline.device.connected = false
    let offlineMenu = MenuState.derive(status: offline, daemonAlive: true)
    check(offlineMenu.icon == .offline && offlineMenu.deviceLine == "未找到盒子", "离线菜单")
    let down = MenuState.derive(status: nil, daemonAlive: false)
    check(down.icon == .daemonDown && down.deviceLineIsAction, "daemon 异常菜单")
    check(Firmware.updateAvailable(device: status.device.firmwareBuild, bundled: "def5678 2026-09-17 09:00"), "哈希不同可更新")
    check(!Firmware.updateAvailable(device: status.device.firmwareBuild, bundled: "abc1234-dirty 2026-09-16 13:11"), "哈希相同不更新")
    check(!Firmware.updateAvailable(device: nil, bundled: "def5678 x"), "盒子未报构建号不催")
    check(Firmware.foreign(connected: true, device: nil, connectedFor: 6), "连着却沉默过了宽限期：出厂机")
    check(!Firmware.foreign(connected: true, device: nil, connectedFor: 1), "刚连上还在宽限期内不算")
    check(!Firmware.foreign(connected: true, device: "abc 1", connectedFor: 60), "报了构建号就是我们的")
    check(!Firmware.foreign(connected: false, device: nil, connectedFor: 60), "没连着谈不上")
} catch {
    check(false, "状态解码抛错：\(error)")
}

// 1b. 固件包：zip 里带一层目录也找得到；验魔数；build.txt 优先于镜像里的版本串。
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
    check(bare.build == "v9.9.9-dirty", "没有 build.txt 时读镜像版本：\(bare.build)")
    try Data("abc1234 2026-09-22 10:00\n".utf8).write(to: nested.appendingPathComponent(FirmwarePackage.buildName))
    let stamped = try FirmwarePackage.inspect(directory: root)
    check(stamped.build == "abc1234 2026-09-22 10:00", "build.txt 优先：\(stamped.build)")
    check(stamped.app.lastPathComponent == FirmwarePackage.appName, "找到子目录里的 app 镜像")
    try write(FirmwarePackage.partitionTableName, [0x00, 0x00])
    do {
        _ = try FirmwarePackage.inspect(directory: root)
        check(false, "坏分区表应当被拒绝")
    } catch let failure as FirmwarePackage.Failure {
        check(failure == .notAnImage(FirmwarePackage.partitionTableName), "坏分区表报的是它自己：\(failure)")
    }
    try FileManager.default.removeItem(at: nested.appendingPathComponent(FirmwarePackage.appName))
    do {
        _ = try FirmwarePackage.inspect(directory: root)
        check(false, "缺 app 镜像应当被拒绝")
    } catch let failure as FirmwarePackage.Failure {
        check(failure == .missing(FirmwarePackage.appName), "缺 app 镜像报缺文件：\(failure)")
    }
} catch {
    check(false, "状态解码抛错：\(error)")
}

// 2. Hook 配置合并：不动别人的 Hook，旧 Python 条目被替换，移除后干净。
let existing: [String: Any] = [
    "hooks": [
        "PreToolUse": [["matcher": "Bash", "hooks": [["type": "command", "command": "'/Users/x/.codex/hooks/rtk-rewrite.sh'"]]]],
        "Stop": [["hooks": [["type": "command", "command": "/usr/bin/python3 /old/codex-hook.py", "timeout": 2]]]],
    ],
    "model": "gpt-5",
]
let binary = "/Users/x/Library/Application Support/VibeBuddy/bin/vibebuddy-hook"
let installed = HookConfig.install(into: existing, agent: .codex, binary: binary)
check(HookConfig.isInstalled(in: installed, agent: .codex, binary: binary), "装好后检测为已装")
check(!HookConfig.isInstalled(in: existing, agent: .codex, binary: binary), "装前检测为未装")
check((installed["model"] as? String) == "gpt-5", "其它键保留")
let installedHooks = installed["hooks"] as! [String: Any]
let preToolUse = installedHooks["PreToolUse"] as! [[String: Any]]
check(preToolUse.count == 1 && (preToolUse[0]["matcher"] as? String) == "Bash", "别人的 PreToolUse 不动（Codex 事件表里没有它）")
let stop = installedHooks["Stop"] as! [[String: Any]]
let stopCommands = stop.flatMap { ($0["hooks"] as! [[String: Any]]).map { $0["command"] as! String } }
check(stopCommands == ["\"\(binary)\" codex"], "旧的 Python 条目被替换：\(stopCommands)")
check(HookAgent.codex.events.allSatisfy { installedHooks[$0] != nil }, "六个事件都在")
let removed = HookConfig.removed(from: installed)
let removedHooks = removed["hooks"] as! [String: Any]
check(removedHooks.keys.sorted() == ["PreToolUse"], "移除后只剩别人的：\(removedHooks.keys.sorted())")
let diff = HookConfig.describeChange(from: existing, to: installed, agent: .codex)
check(diff.contains("- Stop: /usr/bin/python3 /old/codex-hook.py") && diff.contains("+ Stop: \"\(binary)\" codex"), "差异说明：\(diff)")
check(HookAgent.claude.events.count == 8, "Claude 八个事件")
// Codex 信任提示：配置比最近一次事件新才报警；没收到过事件不下结论；读不到文件时间不报警。
let earlier = Date(timeIntervalSince1970: 1_000)
let later = Date(timeIntervalSince1970: 2_000)
check(HookConfig.codexTrustHint(configModifiedAt: later, lastEvent: nil) == .waitingFirstEvent, "没收到过事件：等待")
check(HookConfig.codexTrustHint(configModifiedAt: later, lastEvent: earlier) == .changedSinceLastEvent(later), "配置比最近事件新：要重新信任")
check(HookConfig.codexTrustHint(configModifiedAt: earlier, lastEvent: later) == .trusted, "改完之后收到过事件：已在运行")
check(HookConfig.codexTrustHint(configModifiedAt: nil, lastEvent: later) == .trusted, "读不到文件时间不报警")

// 3. 语音包解析。
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
    check(parsed.voiceID == "hsiaoyu", "音色 id")
    check(parsed.clips[4] == (256 + 100)..<(256 + 150), "第五句位置")
    check(parsed.previewPCM().count == 150 + 4 * 28_800, "试听拼接长度 \(parsed.previewPCM().count)")
} else {
    check(false, "语音包应能解析")
}
check(VoicePack(data: Data("garbage".utf8)) == nil, "垃圾不是语音包")

if failures > 0 {
    print("\(failures) 处失败")
    exit(1)
}
print("自检通过")
