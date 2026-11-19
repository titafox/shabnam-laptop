import SwiftUI

struct DisplayPane: View {
    @EnvironmentObject private var settings: Settings

    var body: some View {
        Form {
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
