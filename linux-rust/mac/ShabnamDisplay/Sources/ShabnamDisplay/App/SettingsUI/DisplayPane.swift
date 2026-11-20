import SwiftUI

struct DisplayPane: View {
    @EnvironmentObject private var model: AppModel
    @EnvironmentObject private var settings: Settings

    var body: some View {
        Form {
            laptopSection
            Section {
                Picker(L("Resolution"), selection: $settings.displayMode) {
                    ForEach(DisplayMode.allCases, id: \.self) { Text($0.title).tag($0) }
                }
                Note(L("The laptop's panel is always 1920 × 1080. HiDPI modes are drawn at twice the size and scaled to fit; the smaller ones make text and controls larger."))
            }
            Section {
                Picker(L("Frame rate"), selection: $settings.frameRate) {
                    Text(L("%d fps", 60)).tag(60)
                    Text(L("%d fps", 30)).tag(30)
                }
                .pickerStyle(.segmented)
                Picker(L("Picture quality"), selection: $settings.quality) {
                    ForEach(QualityPreset.allCases, id: \.self) { Text($0.title).tag($0) }
                }
                Note(L("The cable carries about 300 Mbit/s. Lower settings only help if the picture stutters."))
                Toggle(L("Show the Mac's pointer on the laptop"), isOn: $settings.showCursor)
            }
            Section {
                Button(L("Arrange Displays…")) { Permissions.openDisplaysSettings() }
                Note(L("Opens System Settings › Displays, where the extra screen can be placed next to the Mac's own."))
            }
        }
        .formStyle(.grouped)
    }
}

extension DisplayPane {
    /// The laptop's own backlight, volume and battery, over the control channel.
    private var laptopSection: some View {
        let laptop = model.laptop
        let control = model.laptopControl
        return Section(L("Shabnam laptop")) {
            LabeledContent(L("Battery"), value: laptop.map { $0.batteryText ?? L("No battery reported") } ?? "—")
            Slider(value: Binding(get: { Double(laptop?.brightness ?? 0) }, set: { control.setBrightness(Int($0.rounded())) }), in: 0...100) {
                Text(L("Brightness"))
            } minimumValueLabel: {
                Image(systemName: "sun.min")
            } maximumValueLabel: {
                Image(systemName: "sun.max")
            }
            Slider(value: Binding(get: { Double(laptop?.volume ?? 0) }, set: { control.setVolume(Int($0.rounded())) }),
                   in: 0...Double(laptop?.volumeSteps ?? 16), step: 1) {
                Text(L("Volume"))
            } minimumValueLabel: {
                Image(systemName: "speaker")
            } maximumValueLabel: {
                Image(systemName: "speaker.wave.3")
            }
            Toggle(L("Mute Shabnam"), isOn: Binding(get: { laptop?.muted ?? false }, set: { control.setMuted($0) }))
            HStack {
                Button(L("Return Shabnam to its own desktop")) { control.returnToDesktop() }
                Spacer()
                if let laptop {
                    Text(L("Receiver %@", laptop.receiverVersion) + (laptop.lidClosed == true ? " · " + L("lid closed") : ""))
                        .font(.callout).foregroundStyle(.secondary)
                }
            }
            if laptop == nil {
                Note(model.state.isConnected
                    ? L("This laptop's receiver does not offer these controls yet; it needs a newer subscreen-rx.")
                    : L("Available while the laptop is connected as a second screen."))
            } else {
                Note(L("The laptop's own brightness and volume keys keep working; changes made there show up here."))
            }
        }
        .disabled(laptop == nil)
    }
}

extension DisplayMode {
    var title: String {
        switch self {
        case .hiDPI1080: return L("1920 × 1080 HiDPI (sharper)")
        case .plain1080: return L("1920 × 1080")
        case .hiDPI900: return L("1600 × 900 HiDPI (larger text)")
        case .hiDPI720: return L("1280 × 720 HiDPI (largest text)")
        }
    }
}

extension QualityPreset {
    var title: String {
        let numbers = L("%d Mbit/s, max QP %d", averageBitRate / 1_000_000, maxQP)
        switch self {
        case .sharpest: return L("Sharpest") + " — " + numbers
        case .balanced: return L("Balanced") + " — " + numbers
        case .economy: return L("Economy") + " — " + numbers
        }
    }
}
