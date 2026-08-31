import AppKit
import CowboyInstallerCore
import SwiftUI

struct SettingsView: View {
    @EnvironmentObject private var model: AppModel
    @EnvironmentObject private var launchAtLogin: LaunchAtLoginController
    @State private var showStopConfirmation = false

    var body: some View {
        Form {
            Section("Cowboy Service") {
                TextField("Service URL", text: stringSettingBinding(\.controllerURL))
                    .textContentType(.URL)
                Text("Used for Open Cowboy, account status, and dependency management.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            Section("Updates") {
                Toggle(
                    "Automatically check dependency status",
                    isOn: boolSettingBinding(\.automaticallyCheckForUpdates)
                )
                Text("This manager checks without installing. Cowboy Machine still applies signed automatic component generations through its existing control channel.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            Section("Session security") {
                if let refresh = model.accountStatus.passkeySessionRefresh {
                    Toggle("Refresh browser sessions with a Passkey", isOn: passkeyRefreshBinding)
                        .disabled(refresh.registeredCount == 0)
                    Picker("Refresh frequency", selection: passkeyIntervalBinding) {
                        Text("Every hour").tag(Int64(3_600_000))
                        Text("Every 2 hours").tag(Int64(7_200_000))
                        Text("Every 3 hours").tag(Int64(10_800_000))
                        Text("Every 4 hours").tag(Int64(14_400_000))
                        Text("Every 6 hours").tag(Int64(21_600_000))
                        Text("Every 12 hours").tag(Int64(43_200_000))
                        Text("Every day · Default").tag(Int64(86_400_000))
                        Text("Every 2 days").tag(Int64(172_800_000))
                        Text("Every 3 days").tag(Int64(259_200_000))
                    }
                    .disabled(refresh.registeredCount == 0)
                    Text("Off by default. Periodic verification defaults to one day and the Cowboy Service may require it sooner, up to its three-day maximum. Cowboy Manager keeps its separate one-day session and can sign in again from macOS Keychain.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    if refresh.registeredCount == 0 {
                        Text("Add a Passkey in Cowboy before enabling refresh.")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    } else {
                        LabeledContent("Registered Passkeys", value: String(refresh.registeredCount))
                    }
                    if model.controllerURL != nil {
                        Button("Verify or Manage Passkeys in Cowboy") {
                            CowboyOpener.open(controllerURL: model.controllerURL)
                        }
                    }
                } else {
                    Text("Sign in to manage optional Passkey session refresh.")
                        .foregroundStyle(.secondary)
                }
                if let error = model.accountStatus.errorMessage {
                    Text(error)
                        .foregroundStyle(.red)
                        .textSelection(.enabled)
                }
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

            Section("Background") {
                Toggle("Run Cowboy Machine in background", isOn: machineBackgroundBinding)
                    .disabled(
                        !model.installedStatus.isInstalled
                            || model.serviceActionState.phase.isRunning
                    )
                Text("The Machine is a separate user LaunchAgent and keeps running when this menu app quits.")
                    .font(.caption)
                    .foregroundStyle(.secondary)

                Toggle("Show Cowboy in the menu bar at login", isOn: launchBinding)
                    .disabled(launchAtLogin.status == .unavailable)
                Text(launchStatusDescription)
                    .font(.caption)
                    .foregroundStyle(launchAtLogin.errorMessage == nil ? Color.secondary : Color.red)
                    .textSelection(.enabled)
                Button("Refresh Login Item Status") { launchAtLogin.refresh() }
            }

            Section("About") {
                LabeledContent("Application", value: "Cowboy Manager")
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
            model.refreshAll()
        }
        .confirmationDialog(
            "Stop Cowboy Machine?",
            isPresented: $showStopConfirmation,
            titleVisibility: .visible
        ) {
            Button("Stop Machine", role: .destructive) { model.stopMachine() }
            Button("Cancel", role: .cancel) {}
        } message: {
            if let count = model.remoteMachine?.activeSessions, count > 0 {
                Text("\(count) active session\(count == 1 ? "" : "s") will disconnect until the Machine starts again.")
            } else {
                Text("New sessions cannot run on this Mac until the Machine starts again.")
            }
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

    private var passkeyRefreshBinding: Binding<Bool> {
        Binding(
            get: { model.accountStatus.passkeySessionRefresh?.enabled ?? false },
            set: { enabled in
                guard let refresh = model.accountStatus.passkeySessionRefresh else { return }
                _ = model.setPasskeySessionRefresh(
                    enabled: enabled,
                    intervalMilliseconds: refresh.intervalMilliseconds
                )
            }
        )
    }

    private var passkeyIntervalBinding: Binding<Int64> {
        Binding(
            get: {
                model.accountStatus.passkeySessionRefresh?.intervalMilliseconds ?? 86_400_000
            },
            set: { interval in
                guard let refresh = model.accountStatus.passkeySessionRefresh else { return }
                _ = model.setPasskeySessionRefresh(
                    enabled: refresh.enabled,
                    intervalMilliseconds: interval
                )
            }
        )
    }

    private var launchBinding: Binding<Bool> {
        Binding(
            get: { launchAtLogin.isEnabled },
            set: { launchAtLogin.setEnabled($0) }
        )
    }

    private var machineBackgroundBinding: Binding<Bool> {
        Binding(
            get: { model.isMachineRunning },
            set: { enabled in
                if enabled {
                    model.startMachine()
                } else {
                    showStopConfirmation = true
                }
            }
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
        return switch launchAtLogin.status {
        case .enabled:
            "The lightweight menu app starts at login."
        case .disabled:
            "The menu app does not start automatically."
        case .requiresApproval:
            "macOS requires approval in System Settings → General → Login Items."
        case .unavailable:
            "Install Cowboy Manager in /Applications before enabling Launch at Login."
        }
    }
}
