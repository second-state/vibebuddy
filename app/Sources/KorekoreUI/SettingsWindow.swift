import AppKit
import SwiftUI

/// A korekore window: no system frame, so the view draws the design's own (cream, 3 px ink outline, radius 20,
/// solid shadow); the real window buttons move to where the design draws them.
public final class KorekoreSettingsWindow: NSWindow {
    /// Where the design puts the first button, from the window's top-left corner.
    private let buttonsInset: NSPoint
    private var resizeObserver: Any?

    /// The settings window: 960 × 772, down to 900 × 760, buttons in the sidebar's top row.
    public convenience init<Root: View>(rootView: Root) {
        self.init(rootView: rootView, size: NSSize(width: 960, height: 772), minimum: NSSize(width: 900, height: 760),
                  buttonsInset: NSPoint(x: 17, y: 17), resizable: true)
    }

    /// The onboarding window: the design's 912 × 592, fixed, buttons in its 40 pt top strip.
    public static func onboarding<Root: View>(rootView: Root) -> KorekoreSettingsWindow {
        KorekoreSettingsWindow(rootView: rootView, size: NSSize(width: 912, height: 592), minimum: NSSize(width: 912, height: 592),
                               buttonsInset: NSPoint(x: 16, y: 13), resizable: false)
    }

    public init<Root: View>(rootView: Root, size: NSSize, minimum: NSSize, buttonsInset: NSPoint, resizable: Bool) {
        self.buttonsInset = buttonsInset
        let shadow = KorekoreSettings<EmptyView>.shadow
        super.init(contentRect: NSRect(x: 0, y: 0, width: size.width, height: size.height + shadow),
                   styleMask: NSWindow.StyleMask([.titled, .closable, .miniaturizable, .fullSizeContentView]).union(resizable ? .resizable : []),
                   backing: .buffered, defer: false)
        title = "korekore"
        titleVisibility = .hidden
        titlebarAppearsTransparent = true
        isOpaque = false
        backgroundColor = .clear
        // The design's shadow is drawn by the view; the system's would trace the transparent corners.
        hasShadow = false
        isMovableByWindowBackground = true
        let hosting = NSHostingView(rootView: rootView.frame(minWidth: minimum.width, minHeight: minimum.height + shadow)
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
        let height = buttonsInset.y * 2 + (buttons.first?.frame.height ?? 14)
        if container.frame.height != height {
            container.frame = NSRect(x: 0, y: frame.height - height, width: frame.width, height: height)
        }
        for (index, button) in buttons.enumerated() {
            let superview = button.superview!
            superview.frame = container.bounds
            button.setFrameOrigin(NSPoint(x: buttonsInset.x + CGFloat(index) * (button.frame.width + 6),
                                          y: superview.bounds.height - buttonsInset.y - button.frame.height))
        }
    }
}

