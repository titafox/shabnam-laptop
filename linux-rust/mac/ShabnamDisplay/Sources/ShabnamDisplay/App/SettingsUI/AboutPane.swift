import AppKit
import SwiftUI

struct AboutPane: View {
    var body: some View {
        VStack(spacing: 10) {
            Image(nsImage: NSApp.applicationIconImage)
                .resizable()
                .frame(width: 96, height: 96)
            Text(ProjectInfo.name).font(.title2).fontWeight(.semibold)
            Text(L("Version %@ (%@)", ProjectInfo.version, ProjectInfo.build)).foregroundStyle(.secondary)
            Text(L("Turns a Shabnam laptop into a second screen for this Mac over a USB-C cable."))
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
            Text(L("Licence: %@", ProjectInfo.licence) + " · " + ProjectInfo.copyright)
                .font(.callout)
                .foregroundStyle(.secondary)
            if let homepage = ProjectInfo.homepage {
                SwiftUI.Link(L("Project page"), destination: homepage)
            } else {
                Text(L("Project page: not published yet")).font(.callout).foregroundStyle(.secondary)
            }
        }
        .padding(24)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}
