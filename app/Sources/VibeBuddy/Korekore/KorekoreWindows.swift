import AppKit
import Combine
import KorekoreUI
import SwiftUI
import VibeBuddyCore

/// The settings window with the AppModel behind it.
struct KorekoreSettingsRoot: View {
    @ObservedObject var model: AppModel
    @State private var selection = KorekoreSection.character

    var body: some View {
        // Voice commands are experimental: no Conversation section while the daemon has them off.
        let sections = KorekoreSection.allCases.filter { $0 != .conversation || model.residentEnabled }
        KorekoreSettings(picture: BoxPicture.derive(status: model.status, daemonAlive: model.daemonAlive),
                         versions: KorekoreVersions(app: Resources.bundleVersion, firmware: model.status?.device.firmwareVersion),
                         sections: sections, selection: $selection) { _ in
            KorekorePagePending()
        }
    }
}

/// The menu bar item and the panel it opens (canvas: "Mac · Menu bar"); the same `AppModel` feeds both.
@MainActor
final class KorekoreStatusItem {
    private let statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
    private let model: AppModel
    private let actions: KorekoreMenuActions
    private var panel: NSPanel?
    private var outsideClicks: Any?
    private var subscriptions: Set<AnyCancellable> = []

    init(model: AppModel, actions: KorekoreMenuActions) {
        self.model = model
        self.actions = actions
        statusItem.button?.target = self
        statusItem.button?.action = #selector(toggle)
        model.$status.combineLatest(model.$daemonAlive).receive(on: RunLoop.main).sink { [weak self] status, alive in
            self?.statusItem.button?.image = KorekoreMenuIcon.image(for: BoxPicture.derive(status: status, daemonAlive: alive))
        }.store(in: &subscriptions)
    }

    @objc private func toggle() {
        if panel != nil { close() } else { open() }
    }

    private func open() {
        guard let button = statusItem.button, let buttonWindow = button.window else { return }
        let close = { [weak self] (action: @escaping () -> Void) in { self?.close(); action() } }
        let actions = KorekoreMenuActions(settings: close(actions.settings), checkForUpdates: close(actions.checkForUpdates),
                                          reportProblem: close(actions.reportProblem), quit: actions.quit,
                                          restartDaemon: close(actions.restartDaemon))
        let hosting = NSHostingView(rootView: KorekorePanelRoot(model: model, actions: actions))
        let size = hosting.fittingSize
        let anchor = buttonWindow.convertToScreen(button.convert(button.bounds, to: nil))
        let screen = buttonWindow.screen?.visibleFrame ?? .zero
        // Right edge under the item, kept on screen, 6 pt below the menu bar.
        let x = min(anchor.maxX - size.width + 24, screen.maxX - size.width - 8)
        let panel = NSPanel(contentRect: NSRect(x: x, y: anchor.minY - 6 - size.height, width: size.width, height: size.height),
                            styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = false
        panel.level = .popUpMenu
        panel.contentView = hosting
        panel.orderFrontRegardless()
        self.panel = panel
        statusItem.button?.highlight(true)
        // Like a menu: a click anywhere else closes it.
        outsideClicks = NSEvent.addGlobalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown]) { [weak self] _ in
            MainActor.assumeIsolated { self?.close() }
        }
    }

    private func close() {
        panel?.orderOut(nil)
        panel = nil
        statusItem.button?.highlight(false)
        if let outsideClicks { NSEvent.removeMonitor(outsideClicks) }
        outsideClicks = nil
    }
}

private struct KorekorePanelRoot: View {
    @ObservedObject var model: AppModel
    let actions: KorekoreMenuActions

    var body: some View {
        KorekoreMenuPanel(picture: BoxPicture.derive(status: model.status, daemonAlive: model.daemonAlive), actions: actions)
    }
}
