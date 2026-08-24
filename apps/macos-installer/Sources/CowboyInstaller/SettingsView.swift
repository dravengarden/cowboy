import AppKit
import CowboyInstallerCore
import SwiftUI

struct SettingsView: View {
    @EnvironmentObject private var model: AppModel
    @EnvironmentObject private var launchAtLogin: LaunchAtLoginController

    var body: some View {
        Form {
            Section("Updates") {
                Toggle(
                    "Automatically check for updates",
                    isOn: boolSettingBinding(\.automaticallyCheckForUpdates)
                )
                Text("Cowboy Machine applies signed automatic component generations itself. This manager does not create a second auto-install channel.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            Section("Locations") {
                directorySetting(
                    "Workspace",
                    keyPath: \.workspaceDirectory,
                    prompt: "Directory Cowboy sessions may use"
                )
                directorySetting(
                    "Custom state directory",
                    keyPath: \.stateDirectory,
                    prompt: "Automatic Service-scoped location"
                )
            }

            Section("Notifications") {
                Toggle("Installation notifications", isOn: notificationBinding)
                Text("macOS may also require notification permission in System Settings.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            Section("Startup") {
                Toggle("Launch at Login", isOn: launchBinding)
                    .disabled(launchAtLogin.status == .unavailable)
                Text(launchStatusDescription)
                    .font(.caption)
                    .foregroundStyle(launchAtLogin.errorMessage == nil ? .secondary : .red)
                    .textSelection(.enabled)
                Button("Refresh Login Item Status") { launchAtLogin.refresh() }
            }

            Section("About") {
                LabeledContent("Application", value: "Cowboy Installer")
                LabeledContent("Version", value: model.targetVersion)
                LabeledContent("Bundle ID", value: Bundle.main.bundleIdentifier ?? "development")
                Link("Cowboy project", destination: URL(string: "https://github.com/dravengarden/cowboy")!)
            }
        }
        .formStyle(.grouped)
        .padding(12)
        .navigationTitle("Settings")
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            launchAtLogin.refresh()
        }
    }

    private func boolSettingBinding(_ keyPath: WritableKeyPath<InstallerSettings, Bool>) -> Binding<Bool> {
        Binding(
            get: { model.settings[keyPath: keyPath] },
            set: { newValue in model.updateSettings { $0[keyPath: keyPath] = newValue } }
        )
    }

    private var notificationBinding: Binding<Bool> {
        Binding(
            get: { model.settings.notificationsEnabled },
            set: { enabled in
                if enabled {
                    Task {
                        let authorized = await model.requestNotificationAuthorization()
                        model.updateSettings { $0.notificationsEnabled = authorized }
                    }
                } else {
                    model.updateSettings { $0.notificationsEnabled = false }
                }
            }
        )
    }

    private var launchBinding: Binding<Bool> {
        Binding(
            get: { launchAtLogin.isEnabled },
            set: { launchAtLogin.setEnabled($0) }
        )
    }

    private func stringSettingBinding(_ keyPath: WritableKeyPath<InstallerSettings, String>) -> Binding<String> {
        Binding(
            get: { model.settings[keyPath: keyPath] },
            set: { value in model.updateSettings { $0[keyPath: keyPath] = value } }
        )
    }

    private func directorySetting(
        _ title: String,
        keyPath: WritableKeyPath<InstallerSettings, String>,
        prompt: String
    ) -> some View {
        HStack {
            TextField(title, text: stringSettingBinding(keyPath), prompt: Text(prompt))
            Button("Choose…") {
                let panel = NSOpenPanel()
                panel.canChooseDirectories = true
                panel.canChooseFiles = false
                panel.allowsMultipleSelection = false
                if panel.runModal() == .OK, let url = panel.url {
                    model.updateSettings { $0[keyPath: keyPath] = url.path }
                }
            }
        }
    }

    private var launchStatusDescription: String {
        if let error = launchAtLogin.errorMessage {
            return error
        }
        switch launchAtLogin.status {
        case .enabled:
            "Registered with macOS as a login item."
        case .disabled:
            "Not registered as a login item."
        case .requiresApproval:
            "macOS requires approval in System Settings → General → Login Items."
        case .unavailable:
            "Install Cowboy Installer in /Applications before enabling Launch at Login."
        }
    }
}
