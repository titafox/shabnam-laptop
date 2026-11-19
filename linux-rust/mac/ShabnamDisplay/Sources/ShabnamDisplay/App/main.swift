import AppKit

// A menu-bar-only app: no Dock icon, no main window (LSUIElement in Info.plist; the activation
// policy is set here as well so the bare binary behaves the same when run outside the bundle).
let app = NSApplication.shared
let delegate = AppDelegate()
app.delegate = delegate
app.setActivationPolicy(.accessory)
app.run()
