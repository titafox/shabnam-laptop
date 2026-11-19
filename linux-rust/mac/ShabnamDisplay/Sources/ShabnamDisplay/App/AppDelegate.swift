import AppKit

final class AppDelegate: NSObject, NSApplicationDelegate {
    private let session = SessionController()
    private var statusItem: StatusItemController!

    func applicationDidFinishLaunching(_ notification: Notification) {
        statusItem = StatusItemController(session: session)
        session.onStateChange = { [weak self] state in self?.statusItem.show(state) }
        if Settings.shared.autoConnect { session.start() }
        // First run, or the app's signature changed: explain what is needed instead of failing quietly.
        if !Permissions.screenRecordingGranted {
            // Asking once is what makes macOS list the app under Screen Recording at all;
            // without it the user has to add the app by hand with the "+" button.
            Permissions.requestScreenRecording()
            statusItem.showPermissions()
        }
    }

    func applicationWillTerminate(_ notification: Notification) {
        session.stop()
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }
}
