import AppKit

// A menu-bar-only app: no Dock icon, no main window (LSUIElement in Info.plist; the activation
// policy is set here as well so the bare binary behaves the same when run outside the bundle).
//
// Developer switches:
//   --ui-preview [pane]      only open the Settings window: nothing is connected, no display is
//                            created, a running copy is left alone, settings go to a scratch domain
//   --probe-display-modes    create a short-lived test display and report which modes macOS accepts
let app = NSApplication.shared
app.setActivationPolicy(.accessory)

let delegate = AppDelegate()
app.delegate = delegate
app.run()
