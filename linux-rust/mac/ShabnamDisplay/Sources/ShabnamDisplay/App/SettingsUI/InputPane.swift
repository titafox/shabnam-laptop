import SwiftUI

struct InputPane: View {
    @EnvironmentObject private var model: AppModel
    @EnvironmentObject private var settings: Settings

    var body: some View {
        Form {
            Section {
                Toggle(L("Let the laptop's keyboard and touchpad control this Mac"), isOn: $settings.acceptInput)
                HStack {
                    StatusLight(level: model.accessibilityGranted ? .good : .warning,
                                text: model.accessibilityGranted ? L("Accessibility: granted") : L("Accessibility: not granted — macOS ignores the laptop's input"))
                    Spacer()
                    if !model.accessibilityGranted {
                        Button(L("Open Settings…")) {
                            Permissions.requestAccessibility()
                            Permissions.openAccessibilitySettings()
                        }
                    }
                }
            }
            Section(L("Keyboard")) {
                Picker(L("Modifier keys"), selection: $settings.swapWinAlt) {
                    Text(L("Win → Command, Alt → Option")).tag(false)
                    Text(L("Win → Option, Alt → Command")).tag(true)
                }
                Note(L("Ctrl is always Control."))
                Toggle(L("Play/pause, next and previous keys control the Mac"), isOn: $settings.mediaKeys)
            }
            Section(L("Touchpad")) {
                speed(L("Pointer speed"), $settings.pointerSpeed)
                speed(L("Scroll speed"), $settings.scrollSpeed)
                Toggle(L("Reverse scroll direction"), isOn: $settings.reverseScroll)
            }
            .disabled(!settings.acceptInput)
        }
        .formStyle(.grouped)
    }

    private func speed(_ title: String, _ value: Binding<Double>) -> some View {
        HStack {
            Slider(value: value, in: Settings.speedRange) { Text(title) }
            Text(String(format: "×%.2f", value.wrappedValue))
                .monospacedDigit()
                .foregroundStyle(.secondary)
                .frame(width: 48, alignment: .trailing)
        }
    }
}
