import AppKit
import SwiftUI

/// The korekore settings window: no system frame, so the view draws the design's own (cream, 3 px ink outline,
/// radius 20, solid shadow); the real window buttons move into the sidebar's top row.
public final class KorekoreSettingsWindow: NSWindow {
    /// Where the design puts the first button: the sidebar's padding inside the window's outline.
    private static let buttonsInset = NSPoint(x: 17, y: 17)
    private var resizeObserver: Any?

    public init<Root: View>(rootView: Root) {
        super.init(contentRect: NSRect(x: 0, y: 0, width: 960, height: 772 + KorekoreSettings<EmptyView>.shadow),
                   styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
                   backing: .buffered, defer: false)
        title = "korekore"
        titleVisibility = .hidden
        titlebarAppearsTransparent = true
        isOpaque = false
        backgroundColor = .clear
        // The design's shadow is drawn by the view; the system's would trace the transparent corners.
        hasShadow = false
        isMovableByWindowBackground = true
        let hosting = NSHostingView(rootView: rootView.frame(minWidth: 900, minHeight: 760 + KorekoreSettings<EmptyView>.shadow)
            .ignoresSafeArea())
        hosting.sizingOptions = [.minSize]
        // The view runs up under the (invisible) title bar: the design's window has none.
        hosting.safeAreaRegions = []
        contentView = hosting
        resizeObserver = NotificationCenter.default.addObserver(forName: NSWindow.didResizeNotification, object: self, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.placeButtons() }
        }
        placeButtons()
    }

    override public func layoutIfNeeded() {
        super.layoutIfNeeded()
        placeButtons()
    }

    /// AppKit lays the buttons out again on resize and on key changes, so this runs after each.
    private func placeButtons() {
        let buttons = [NSWindow.ButtonType.closeButton, .miniaturizeButton, .zoomButton].compactMap { standardWindowButton($0) }
        guard let container = buttons.first?.superview?.superview else { return }
        // The title bar is only 28 pt tall; make room for the design's row, 17 pt in from the outline.
        let height = Self.buttonsInset.y * 2 + (buttons.first?.frame.height ?? 14)
        if container.frame.height != height {
            container.frame = NSRect(x: 0, y: frame.height - height, width: frame.width, height: height)
        }
        for (index, button) in buttons.enumerated() {
            let superview = button.superview!
            superview.frame = container.bounds
            button.setFrameOrigin(NSPoint(x: Self.buttonsInset.x + CGFloat(index) * (button.frame.width + 6),
                                          y: superview.bounds.height - Self.buttonsInset.y - button.frame.height))
        }
    }
}

