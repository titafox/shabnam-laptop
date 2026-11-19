import SwiftUI

struct GeneralPane: View {
    @EnvironmentObject private var model: AppModel
    @EnvironmentObject private var settings: Settings

    var body: some View {
        Form {
            Section {
                Toggle(L("Launch at login"), isOn: Binding(get: { model.loginItemEnabled }, set: { model.setLoginItem($0) }))
                Toggle(L("Connect automatically when the laptop appears"), isOn: $settings.autoConnect)
            }
            Section(L("Menu bar")) {
                Picker(L("Show"), selection: $settings.menuBarShowsDetails) {
                    Text(L("Icon only")).tag(false)
                    Text(L("Icon and bitrate while connected")).tag(true)
                }
                HStack(spacing: 18) {
                    legend(.off, L("Off"))
                    legend(.idle, L("Waiting"))
                    legend(.active, L("Connected"))
                    legend(.attention, L("Problem"))
                }
            }
            Section {
                Toggle(L("Notify when the laptop connects or disconnects"), isOn: $settings.notifications)
            }
        }
        .formStyle(.grouped)
    }

    private func legend(_ style: StatusIcon.Style, _ text: String) -> some View {
        HStack(spacing: 5) {
            Image(nsImage: StatusIcon.image(style))
            Text(text).font(.callout).foregroundStyle(.secondary)
        }
    }
}
