import AppKit
import CowboyInstallerCore
import SwiftUI

@main
struct CowboyInstallerApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    @StateObject private var model: AppModel
    @StateObject private var launchAtLogin: LaunchAtLoginController

    init() {
        let runtime = AppRuntime.live()
        _model = StateObject(wrappedValue: runtime.model)
        _launchAtLogin = StateObject(wrappedValue: runtime.launchAtLogin)
    }

    var body: some Scene {
        MenuBarExtra {
            InstallerMenuView()
                .environmentObject(model)
                .environmentObject(launchAtLogin)
        } label: {
            Label("Cowboy Installer", systemImage: model.installState.menuBarSymbol)
                .accessibilityLabel("Cowboy Installer, \(model.installState.message)")
        }

        Window("Cowboy Installer", id: AppWindow.managerID) {
            ManagerRootView()
                .environmentObject(model)
                .environmentObject(launchAtLogin)
                .onAppear {
                    model.refreshInstalledStatus()
                    launchAtLogin.refresh()
                }
        }
        .defaultSize(width: 820, height: 560)
        .windowResizability(.contentMinSize)

        Settings {
            SettingsView()
                .environmentObject(model)
                .environmentObject(launchAtLogin)
                .frame(width: 520, height: 420)
                .onAppear { launchAtLogin.refresh() }
        }
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationShouldTerminateAfterLastWindowClosed(_: NSApplication) -> Bool {
        false
    }
}

@MainActor
enum AppWindow {
    static let managerID = "cowboy-installer-manager"

    static func activateExistingManager() {
        NSApplication.shared.activate()
        NSApplication.shared.windows
            .first(where: { $0.title == "Cowboy Installer" })?
            .makeKeyAndOrderFront(nil)
    }
}

extension InstallState {
    var menuBarSymbol: String {
        switch phase {
        case .preparing, .validating, .installing, .activating, .refreshing:
            "arrow.down.circle"
        case .succeeded:
            "checkmark.circle.fill"
        case .failed, .interrupted:
            "exclamationmark.triangle.fill"
        case .cancelled:
            "xmark.circle.fill"
        case .idle:
            "shippingbox"
        }
    }

    var statusColor: Color {
        switch phase {
        case .succeeded: .green
        case .failed, .interrupted: .orange
        case .cancelled: .secondary
        case .preparing, .validating, .installing, .activating, .refreshing: .accentColor
        case .idle: .secondary
        }
    }
}
