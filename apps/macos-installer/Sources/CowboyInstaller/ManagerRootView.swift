import CowboyInstallerCore
import SwiftUI

@MainActor
final class ManagerNavigationModel: ObservableObject {
    @Published var selection: ManagerSection? = .dashboard
}

enum ManagerSection: String, CaseIterable, Identifiable {
    case dashboard = "Dashboard"
    case install = "Install"
    case activity = "Activity"
    case account = "Account"
    case settings = "Settings"

    var id: String { rawValue }

    var symbol: String {
        switch self {
        case .dashboard: "gauge.with.dots.needle.67percent"
        case .install: "shippingbox.and.arrow.backward"
        case .activity: "clock.arrow.circlepath"
        case .account: "person.crop.circle"
        case .settings: "gearshape"
        }
    }
}

struct ManagerRootView: View {
    @EnvironmentObject private var navigation: ManagerNavigationModel

    var body: some View {
        NavigationSplitView {
            List(ManagerSection.allCases, selection: $navigation.selection) { section in
                Label(section.rawValue, systemImage: section.symbol)
                    .tag(section)
            }
            .navigationSplitViewColumnWidth(min: 160, ideal: 180)
        } detail: {
            switch navigation.selection ?? .dashboard {
            case .dashboard:
                DashboardView()
            case .install:
                InstallView()
            case .activity:
                ActivityView()
            case .account:
                AccountView()
            case .settings:
                SettingsView()
            }
        }
        .frame(minWidth: 720, minHeight: 480)
    }
}
