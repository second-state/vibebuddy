import AppKit

// AppKit 生命周期：菜单栏常驻、不上 Dock（Info.plist 里 LSUIElement 为真）。
let application = NSApplication.shared
let delegate = MainActor.assumeIsolated { AppDelegate() }
application.delegate = delegate
application.setActivationPolicy(.accessory)
application.run()
