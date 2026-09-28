import AppKit

// AppKit lifecycle: lives in the menu bar, no Dock icon (LSUIElement is true in Info.plist).
let application = NSApplication.shared
let delegate = MainActor.assumeIsolated { AppDelegate() }
application.delegate = delegate
application.setActivationPolicy(.accessory)
application.run()
