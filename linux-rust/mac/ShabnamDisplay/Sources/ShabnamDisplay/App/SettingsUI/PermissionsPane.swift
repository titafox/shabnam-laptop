import SwiftUI

/// Shows whether the macOS permissions are granted and leads to the right panes of System
/// Settings. The app never tries to work around these prompts.
struct PermissionsPane: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        Form {
            Section {
                row(L("Screen Recording"),
                    L("To capture the extra screen and the Mac's sound and send them to the laptop."),
                    light: StatusLight(level: model.screenRecordingGranted ? .good : .warning, text: model.screenRecordingGranted ? L("Granted") : L("Not granted"))) {
                    if !model.screenRecordingGranted { Permissions.requestScreenRecording() }
                    Permissions.openScreenRecordingSettings()
                }
                row(L("System Audio Recording"),
                    L("Only for the sound mode “Shabnam only”. macOS does not let apps check this one; it asks the first time the mode is used."),
                    light: StatusLight(level: .unknown, text: L("Asked on first use"))) {
                    Permissions.openAudioRecordingSettings()
                }
                row(L("Accessibility"),
                    L("To let the laptop's keyboard and touchpad control this Mac."),
                    light: StatusLight(level: model.accessibilityGranted ? .good : .warning, text: model.accessibilityGranted ? L("Granted") : L("Not granted"))) {
                    if !model.accessibilityGranted { Permissions.requestAccessibility() }
                    Permissions.openAccessibilitySettings()
                }
            }
            Section {
                HStack {
                    Note(L("After allowing Screen Recording, macOS may ask you to quit and reopen the app."))
                    Spacer()
                    Button(L("Check Again")) { model.refresh() }
                }
            }
        }
        .formStyle(.grouped)
    }

    private func row(_ title: String, _ detail: String, light: StatusLight, action: @escaping () -> Void) -> some View {
        HStack(alignment: .center) {
            VStack(alignment: .leading, spacing: 3) {
                HStack(spacing: 10) {
                    Text(title).fontWeight(.medium)
                    light
                }
                Note(detail)
            }
            Spacer()
            Button(L("Open Settings…"), action: action)
        }
    }
}
