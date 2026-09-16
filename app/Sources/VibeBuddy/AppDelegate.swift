import AppKit
import Combine
import SwiftUI
import VibeBuddyCore

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private let model = AppModel()
    private var statusItem: NSStatusItem!
    private var settingsWindow: NSWindow?
    private var onboardingWindow: NSWindow?
    private var subscriptions: Set<AnyCancellable> = []

    private var terminationSignal: DispatchSourceSignal?

    func applicationDidFinishLaunching(_ notification: Notification) {
        // `kill`/`pkill` 发来的 SIGTERM 默认直接结束进程，daemon 会变成孤儿；
        // 接住它走正常退出，applicationWillTerminate 才有机会停掉 daemon。
        signal(SIGTERM, SIG_IGN)
        let source = DispatchSource.makeSignalSource(signal: SIGTERM, queue: .main)
        source.setEventHandler { NSApp.terminate(nil) }
        source.resume()
        terminationSignal = source

        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        statusItem.button?.image = PixelFace.image(eyesClosed: true)
        statusItem.menu = NSMenu()
        model.$menu.receive(on: RunLoop.main).sink { [weak self] menu in self?.render(menu) }.store(in: &subscriptions)
        model.$lastError.receive(on: RunLoop.main).compactMap { $0 }.sink { message in
            let alert = NSAlert()
            alert.messageText = "Vibe Buddy"
            alert.informativeText = message
            alert.runModal()
        }.store(in: &subscriptions)

        Task { @MainActor in
            await LegacyLaunchAgent.migrateIfNeeded(model: model)
            model.start()
        }
        if !UserDefaults.standard.bool(forKey: "onboardingDone") {
            showOnboarding()
        }
    }

    func applicationWillTerminate(_ notification: Notification) {
        model.shutdown()
    }

    private func render(_ state: MenuState) {
        statusItem.button?.image = PixelFace.image(eyesClosed: state.icon != .online)
        // 链路断开或 daemon 没起来都灰掉：闭眼加变灰才是"它不在"。
        statusItem.button?.appearsDisabled = state.icon != .online
        let menu = NSMenu()
        let device = NSMenuItem(title: state.deviceLine, action: state.deviceLineIsAction ? #selector(restartDaemon) : nil, keyEquivalent: "")
        device.target = self
        device.isEnabled = state.deviceLineIsAction
        menu.addItem(device)
        let mode = NSMenuItem(title: state.modeLine, action: nil, keyEquivalent: "")
        mode.isEnabled = false
        menu.addItem(mode)
        let today = NSMenuItem(title: state.todayLine, action: nil, keyEquivalent: "")
        today.isEnabled = false
        menu.addItem(today)
        menu.addItem(.separator())
        let settings = NSMenuItem(title: "设置…", action: #selector(showSettings), keyEquivalent: ",")
        settings.target = self
        menu.addItem(settings)
        menu.addItem(.separator())
        let quit = NSMenuItem(title: "退出 Vibe Buddy（盒子将离线）", action: #selector(quit), keyEquivalent: "q")
        quit.target = self
        menu.addItem(quit)
        statusItem.menu = menu
    }

    @objc private func restartDaemon() { model.restartDaemon() }

    @objc func showSettings() {
        if settingsWindow == nil {
            let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 640, height: 480), styleMask: [.titled, .closable, .miniaturizable], backing: .buffered, defer: false)
            window.title = "Vibe Buddy 设置"
            window.contentView = NSHostingView(rootView: SettingsView(model: model))
            window.center()
            window.isReleasedWhenClosed = false
            settingsWindow = window
        }
        NSApp.activate(ignoringOtherApps: true)
        settingsWindow?.makeKeyAndOrderFront(nil)
    }

    func showOnboarding() {
        if onboardingWindow == nil {
            let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 560, height: 440), styleMask: [.titled, .closable], backing: .buffered, defer: false)
            window.title = "欢迎使用 Vibe Buddy"
            window.contentView = NSHostingView(rootView: OnboardingView(model: model, finish: { [weak self] in
                UserDefaults.standard.set(true, forKey: "onboardingDone")
                self?.onboardingWindow?.close()
            }))
            window.center()
            window.isReleasedWhenClosed = false
            onboardingWindow = window
        }
        NSApp.activate(ignoringOtherApps: true)
        onboardingWindow?.makeKeyAndOrderFront(nil)
    }

    @objc private func quit() { NSApp.terminate(nil) }
}

/// 旧的 LaunchAgent 时代：发现它就提议卸掉并接管，两套 daemon 不能同时抢串口。
enum LegacyLaunchAgent {
    static let label = "com.agentbeacon.beacond"
    static var plist: URL {
        FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/LaunchAgents/\(label).plist")
    }

    /// 7331 上已经有 daemon 在应答：不管是谁起的，都不能再拉一个。
    static func portOccupied() async -> Bool {
        var request = URLRequest(url: URL(string: "http://127.0.0.1:7331/v1/status")!)
        request.timeoutInterval = 1
        guard let (_, response) = try? await URLSession.shared.data(for: request) else { return false }
        return (response as? HTTPURLResponse)?.statusCode == 200
    }

    @MainActor
    static func migrateIfNeeded(model: AppModel) async {
        let hasPlist = FileManager.default.fileExists(atPath: plist.path)
        let occupied = await portOccupied()
        guard hasPlist || occupied else { return }
        let alert = NSAlert()
        alert.messageText = hasPlist ? "发现旧的 beacond 后台服务" : "7331 端口已经有 daemon 在跑"
        alert.informativeText = "Vibe Buddy 现在自己看管 daemon，两个 daemon 会抢串口。卸掉旧的并接管吗？"
        alert.addButton(withTitle: "卸载并接管")
        alert.addButton(withTitle: "稍后")
        NSApp.activate(ignoringOtherApps: true)
        if alert.runModal() == .alertFirstButtonReturn {
            let bootout = Process()
            bootout.executableURL = URL(fileURLWithPath: "/bin/launchctl")
            bootout.arguments = ["bootout", "gui/\(getuid())/\(label)"]
            try? bootout.run()
            bootout.waitUntilExit()
            if hasPlist { try? FileManager.default.trashItem(at: plist, resultingItemURL: nil) }
            // 不是 LaunchAgent 起的（比如手工 cargo run），请它自己退出。
            if occupied { try? await DaemonClient().restart() }
            model.managesDaemon = true
        } else {
            // 用户留着旧服务：本次不拉自己的 daemon，只跟旧的说话。
            model.managesDaemon = false
        }
    }
}
