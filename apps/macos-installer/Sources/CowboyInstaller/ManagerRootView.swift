import CowboyInstallerCore
import SwiftUI

enum ManagerSection: String, CaseIterable, Identifiable {
    case install = "Install"
    case activity = "Activity"
    case settings = "Settings"

    var id: String { rawValue }

    var symbol: String {
        switch self {
        case .install: "shippingbox.and.arrow.backward"
        case .activity: "clock.arrow.circlepath"
        case .settings: "gearshape"
        }
    }
}

struct ManagerRootView: View {
    @State private var selection: ManagerSection? = .install

    var body: some View {
        NavigationSplitView {
            List(ManagerSection.allCases, selection: $selection) { section in
                Label(section.rawValue, systemImage: section.symbol)
                    .tag(section)
            }
            .navigationSplitViewColumnWidth(min: 160, ideal: 180)
        } detail: {
            switch selection ?? .install {
            case .install:
                InstallView()
            case .activity:
                ActivityView()
            case .settings:
                SettingsView()
            }
        }
        .frame(minWidth: 720, minHeight: 480)
    }
}
