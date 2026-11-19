import AppKit
import ApplicationServices
import CoreGraphics

/// The two macOS privacy permissions the app depends on.
enum Permissions {
    /// Needed to capture the virtual display (and the system sound).
    static var screenRecordingGranted: Bool { CGPreflightScreenCaptureAccess() }

    /// Needed to replay the laptop's keyboard and touchpad; without it macOS silently drops the events.
    static var accessibilityGranted: Bool { AXIsProcessTrusted() }

    /// Shows the system's own prompt (macOS only shows it once per app identity).
    static func requestScreenRecording() { _ = CGRequestScreenCaptureAccess() }

    static func requestAccessibility() {
        _ = AXIsProcessTrustedWithOptions(["AXTrustedCheckOptionPrompt": true] as CFDictionary)
    }

    static func openScreenRecordingSettings() {
        open("x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture")
    }

    static func openAccessibilitySettings() {
        open("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
    }

    private static func open(_ string: String) {
        if let url = URL(string: string) { NSWorkspace.shared.open(url) }
    }
}
