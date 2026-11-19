import SwiftUI

struct AudioPane: View {
    @EnvironmentObject private var settings: Settings

    var body: some View {
        Form {
            Section {
                Picker(L("Sound"), selection: $settings.audioMode) {
                    Text(L("Sound: Mac and Shabnam")).tag(AudioMode.both)
                    if AudioMode.laptopOnlySupported {
                        Text(L("Sound: Shabnam only (Mac muted)")).tag(AudioMode.laptopOnly)
                    }
                    Text(L("Sound: off")).tag(AudioMode.off)
                }
                .pickerStyle(.radioGroup)
                .labelsHidden()
            }
            Section {
                Note(L("Mac and Shabnam: the Mac keeps playing and the laptop plays the same sound."))
                Note(L("Shabnam only: the sound is taken away from the Mac's speakers while the laptop is connected and comes back the moment it disconnects. The Mac's volume keys have no effect on it; use the laptop's volume keys. macOS may ask for permission to record system audio the first time."))
                Note(L("Off: nothing is sent; the Mac plays as usual."))
            }
        }
        .formStyle(.grouped)
    }
}
