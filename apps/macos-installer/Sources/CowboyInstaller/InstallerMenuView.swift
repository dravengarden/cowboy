import AppKit
import CowboyInstallerCore
import SwiftUI

struct InstallerMenuView: View {
    @Environment(\.openWindow) private var openWindow
    @EnvironmentObject private var model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Label(model.installState.message, systemImage: model.installState.menuBarSymbol)
                .foregroundStyle(model.installState.statusColor)
            if model.isRunning {
                ProgressView(value: model.installState.progress)
                Text("\(Int(model.installState.progress * 100))%")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            Divider()

            Button(model.installedStatus.isInstalled ? "Update Cowboy…" : "Install Cowboy…") {
                openManager()
            }
            .keyboardShortcut("i")
            .disabled(model.isRunning)

            if model.isRunning {
                Button("Cancel") { model.cancelInstall() }
            } else if model.canRetry {
                Button("Retry…") { openManager() }
            }

            Divider()

            Button("Open Cowboy Manager") { openManager() }
                .keyboardShortcut("o")
            SettingsLink {
                Text("Settings…")
            }

            Divider()

            Button("Quit Cowboy Installer") { NSApplication.shared.terminate(nil) }
                .keyboardShortcut("q")
                .disabled(model.isRunning)
        }
        .padding(.vertical, 4)
    }

    private func openManager() {
        openWindow(id: AppWindow.managerID)
        DispatchQueue.main.async { AppWindow.activateExistingManager() }
    }
}
