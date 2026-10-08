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
        // A second copy of the app (another path, a dev build) would start a second daemon that fights over
        // the port and the serial port. Hand off to the running one and quit before starting anything.
        if SingleInstance.handOffToRunningCopy() { exit(0) }
        DistributedNotificationCenter.default().addObserver(forName: SingleInstance.showSettings, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.showMainWindow() }
        }
        NSApp.mainMenu = mainMenu()

        // A SIGTERM from `kill`/`pkill` ends the process outright by default, orphaning the daemon;
        // catch it and quit normally so applicationWillTerminate gets a chance to stop the daemon.
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
        } else if CommandLine.arguments.contains(AppRelaunch.showSettingsArgument) {
            showSettings()
        }
    }

    func applicationWillTerminate(_ notification: Notification) {
        model.shutdown()
    }

    /// Double-clicking the app while it runs lands here: open its window rather than doing nothing.
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        showMainWindow()
        return false
    }

    /// Onboarding if it's still open, Settings otherwise.
    private func showMainWindow() {
        if onboardingWindow.map(isOpen) == true { showOnboarding() } else { showSettings() }
    }

    /// Minimized still counts: the Dock icon is how you get the window back.
    private func isOpen(_ window: NSWindow) -> Bool { window.isVisible || window.isMiniaturized }

    /// Shows a window with a Dock icon, so it can be found again; the icon goes when the last window closes.
    private func present(_ window: NSWindow) {
        NSApp.setActivationPolicy(.regular)
        NSApp.activate(ignoringOtherApps: true)
        window.makeKeyAndOrderFront(nil)
    }

    private func watchClose(_ window: NSWindow) {
        NotificationCenter.default.addObserver(forName: NSWindow.willCloseNotification, object: window, queue: .main) { [weak self] _ in
            // willClose fires while the window is still visible; check once it's gone.
            DispatchQueue.main.async {
                guard let self else { return }
                if ![self.settingsWindow, self.onboardingWindow].compactMap({ $0 }).contains(where: self.isOpen) {
                    NSApp.setActivationPolicy(.accessory)
                }
            }
        }
    }

    /// The menu bar while a window is open: without it ⌘Q, ⌘W and copy/paste do nothing.
    private func mainMenu() -> NSMenu {
        let main = NSMenu()
        let app = NSMenu()
        let settings = NSMenuItem(title: String(localized: "Settings…"), action: #selector(showSettings), keyEquivalent: ",")
        settings.target = self
        app.addItem(settings)
        app.addItem(.separator())
        app.addItem(NSMenuItem(title: String(localized: "Hide Vibe Buddy"), action: #selector(NSApplication.hide(_:)), keyEquivalent: "h"))
        app.addItem(.separator())
        let quit = NSMenuItem(title: String(localized: "Quit Vibe Buddy (the box goes offline)"), action: #selector(quit), keyEquivalent: "q")
        quit.target = self
        app.addItem(quit)
        let edit = NSMenu(title: String(localized: "Edit"))
        edit.addItem(NSMenuItem(title: String(localized: "Cut"), action: #selector(NSText.cut(_:)), keyEquivalent: "x"))
        edit.addItem(NSMenuItem(title: String(localized: "Copy"), action: #selector(NSText.copy(_:)), keyEquivalent: "c"))
        edit.addItem(NSMenuItem(title: String(localized: "Paste"), action: #selector(NSText.paste(_:)), keyEquivalent: "v"))
        edit.addItem(NSMenuItem(title: String(localized: "Select All"), action: #selector(NSText.selectAll(_:)), keyEquivalent: "a"))
        let window = NSMenu(title: String(localized: "Window"))
        window.addItem(NSMenuItem(title: String(localized: "Close"), action: #selector(NSWindow.performClose(_:)), keyEquivalent: "w"))
        window.addItem(NSMenuItem(title: String(localized: "Minimize"), action: #selector(NSWindow.performMiniaturize(_:)), keyEquivalent: "m"))
        for submenu in [app, edit, window] {
            let item = NSMenuItem()
            item.submenu = submenu
            main.addItem(item)
        }
        return main
    }

    private func render(_ state: MenuState) {
        statusItem.button?.image = PixelFace.image(eyesClosed: state.icon != .online)
        // Gray out when the link is down or the daemon isn't up: closed eyes plus gray means "it's not there".
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
        let settings = NSMenuItem(title: String(localized: "Settings…"), action: #selector(showSettings), keyEquivalent: ",")
        settings.target = self
        menu.addItem(settings)
        let updates = NSMenuItem(title: String(localized: "Check for Updates…"), action: #selector(checkForUpdates), keyEquivalent: "")
        updates.target = self
        menu.addItem(updates)
        let report = NSMenuItem(title: String(localized: "Report a Problem…"), action: #selector(reportProblem), keyEquivalent: "")
        report.target = self
        menu.addItem(report)
        menu.addItem(.separator())
        let quit = NSMenuItem(title: String(localized: "Quit Vibe Buddy (the box goes offline)"), action: #selector(quit), keyEquivalent: "q")
        quit.target = self
        menu.addItem(quit)
        statusItem.menu = menu
    }

    @objc private func restartDaemon() { model.restartDaemon() }
    @objc private func checkForUpdates() { model.checkForUpdates() }
    @objc private func reportProblem() { model.reportProblem() }

    @objc func showSettings() {
        if settingsWindow == nil {
            // Roomy enough for the whole Character tab at first sight, and resizable down to the old size.
            let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 760, height: 640), styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
            window.title = String(localized: "Vibe Buddy Settings")
            let hosting = NSHostingView(rootView: SettingsView(model: model))
            // Only the minimum comes from SwiftUI; otherwise the view's ideal size would pin the window.
            hosting.sizingOptions = [.minSize]
            window.contentView = hosting
            window.contentMinSize = NSSize(width: 640, height: 480)
            window.center()
            window.isReleasedWhenClosed = false
            watchClose(window)
            settingsWindow = window
        }
        settingsWindow.map(present)
    }

    func showOnboarding() {
        if onboardingWindow == nil {
            let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 560, height: 440), styleMask: [.titled, .closable], backing: .buffered, defer: false)
            window.title = String(localized: "Welcome to Vibe Buddy")
            window.contentView = NSHostingView(rootView: OnboardingView(model: model, finish: { [weak self] in
                UserDefaults.standard.set(true, forKey: "onboardingDone")
                self?.onboardingWindow?.close()
            }))
            window.center()
            window.isReleasedWhenClosed = false
            watchClose(window)
            onboardingWindow = window
        }
        onboardingWindow.map(present)
    }

    @objc private func quit() { NSApp.terminate(nil) }
}

enum SingleInstance {
    static let showSettings = Notification.Name("com.vibebuddy.app.showSettings")

    /// If another copy is already running, ask it to open Settings and return true: this one should quit.
    @MainActor
    static func handOffToRunningCopy() -> Bool {
        guard let bundleID = Bundle.main.bundleIdentifier,
              let other = NSRunningApplication.runningApplications(withBundleIdentifier: bundleID)
                .first(where: { $0.processIdentifier != getpid() }) else { return false }
        DistributedNotificationCenter.default().postNotificationName(showSettings, object: nil, userInfo: nil, deliverImmediately: true)
        // We were just launched by the user, so we may pass activation on to the running copy.
        NSApp.yieldActivation(to: other)
        other.activate()
        return true
    }
}

/// Leftover from the LaunchAgent era: if found, offer to remove it and take over, since two daemons can't share the serial port.
enum LegacyLaunchAgent {
    /// The label from before the rename: it must match what old machines still have, so it can't be renamed.
    static let label = "com.agentbeacon.beacond"
    static var plist: URL {
        FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/LaunchAgents/\(label).plist")
    }

    /// A daemon already answers on 7331: whoever started it, we must not launch another.
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
        alert.messageText = hasPlist ? String(localized: "Found an old vibebuddyd background service") : String(localized: "A daemon is already running on port 7331")
        alert.informativeText = String(localized: "Vibe Buddy now manages its own daemon, and two daemons would fight over the serial port. Remove the old one and take over?")
        alert.addButton(withTitle: String(localized: "Remove and take over"))
        alert.addButton(withTitle: String(localized: "Later"))
        NSApp.activate(ignoringOtherApps: true)
        if alert.runModal() == .alertFirstButtonReturn {
            let bootout = Process()
            bootout.executableURL = URL(fileURLWithPath: "/bin/launchctl")
            bootout.arguments = ["bootout", "gui/\(getuid())/\(label)"]
            try? bootout.run()
            bootout.waitUntilExit()
            if hasPlist { try? FileManager.default.trashItem(at: plist, resultingItemURL: nil) }
            // Not started by the LaunchAgent (e.g. a manual cargo run): ask it to exit itself.
            if occupied { try? await DaemonClient().restart() }
            model.managesDaemon = true
        } else {
            // The user keeps the old service: don't launch our own daemon this time, just talk to the old one.
            model.managesDaemon = false
        }
    }
}
