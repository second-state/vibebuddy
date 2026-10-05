import AppKit

// AppKit lifecycle: lives in the menu bar (LSUIElement is true in Info.plist); a Dock icon appears only while a window is open.
let application = NSApplication.shared
let delegate = MainActor.assumeIsolated { AppDelegate() }
application.delegate = delegate
application.setActivationPolicy(.accessory)
application.run()
