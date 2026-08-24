import AppKit
import CowboyInstallerCore
import SwiftUI

struct InstallView: View {
    @EnvironmentObject private var model: AppModel
    @State private var enrollmentToken = ""

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                header
                statusCard
                configurationCard
                actionRow
            }
            .padding(24)
            .frame(maxWidth: 680, alignment: .leading)
        }
        .navigationTitle("Install")
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 5) {
            Text("Cowboy Machine")
                .font(.largeTitle.weight(.semibold))
            Text("Install the existing user-scoped Cowboy Machine backend and keep it running with a macOS LaunchAgent.")
                .foregroundStyle(.secondary)
        }
    }

    private var statusCard: some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 12) {
                HStack {
                    Label(model.installState.message, systemImage: model.installState.menuBarSymbol)
                        .foregroundStyle(model.installState.statusColor)
                    Spacer()
                    Text(model.installedStatus.isInstalled ? "Installed" : "Not installed")
                        .foregroundStyle(.secondary)
                }
                if model.isRunning {
                    ProgressView(value: model.installState.progress) {
                        Text("\(Int(model.installState.progress * 100))%")
                    }
                    .accessibilityLabel("Installation progress")
                    .accessibilityValue("\(Int(model.installState.progress * 100)) percent")
                }
                LabeledContent("Target version", value: model.targetVersion)
                LabeledContent("Installed version", value: model.installedStatus.version ?? "—")
                LabeledContent("Installation location", value: model.installedStatus.location ?? effectiveStateDirectory)
                if let origin = model.installedStatus.serviceOrigin {
                    LabeledContent("Cowboy Service", value: origin)
                }
                if let error = model.installState.errorMessage {
                    Text(error)
                        .foregroundStyle(.red)
                        .textSelection(.enabled)
                }
            }
            .padding(8)
        }
    }

    private var configurationCard: some View {
        GroupBox("Enrollment") {
            Form {
                TextField("Cowboy Service URL", text: settingBinding(\.controllerURL))
                    .textContentType(.URL)
                SecureField("One-time enrollment code", text: $enrollmentToken)
                    .textContentType(.oneTimeCode)
                    .accessibilityLabel("One-time Cowboy Machine enrollment code")
                directoryRow(
                    title: "Workspace",
                    value: settingBinding(\.workspaceDirectory),
                    emptyLabel: "Choose the directory Cowboy sessions may use"
                )
                directoryRow(
                    title: "Custom state directory",
                    value: settingBinding(\.stateDirectory),
                    emptyLabel: "Automatic Service-scoped location"
                )
                Text("The enrollment code is written to a temporary mode-0600 file, passed with --token-file, and never stored in settings or activity history.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            .formStyle(.grouped)
        }
    }

    private var actionRow: some View {
        HStack {
            Button("Refresh Status") { model.refreshInstalledStatus() }
            Spacer()
            if model.isRunning {
                Button("Cancel", role: .destructive) { model.cancelInstall() }
                    .keyboardShortcut(.cancelAction)
            } else {
                Button(model.installedStatus.isInstalled ? "Update" : "Install") { start() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(enrollmentToken.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }
    }

    private var effectiveStateDirectory: String {
        model.settings.stateDirectory.isEmpty
            ? "~/.local/state/cowboy-machine/services/<service-id>"
            : model.settings.stateDirectory
    }

    private func settingBinding(_ keyPath: WritableKeyPath<InstallerSettings, String>) -> Binding<String> {
        Binding(
            get: { model.settings[keyPath: keyPath] },
            set: { newValue in model.updateSettings { $0[keyPath: keyPath] = newValue } }
        )
    }

    private func directoryRow(
        title: String,
        value: Binding<String>,
        emptyLabel: String
    ) -> some View {
        HStack {
            TextField(title, text: value, prompt: Text(emptyLabel))
            Button("Choose…") {
                let panel = NSOpenPanel()
                panel.canChooseDirectories = true
                panel.canChooseFiles = false
                panel.allowsMultipleSelection = false
                if panel.runModal() == .OK, let url = panel.url {
                    value.wrappedValue = url.path
                }
            }
            .accessibilityLabel("Choose \(title.lowercased())")
        }
    }

    private func start() {
        let token = enrollmentToken
        let request = InstallRequest(
            controllerURL: model.settings.controllerURL,
            enrollmentToken: token,
            workspaceDirectory: model.settings.workspaceDirectory,
            stateDirectory: model.settings.stateDirectory.isEmpty ? nil : model.settings.stateDirectory,
            targetVersion: model.targetVersion
        )
        if model.startInstall(request) {
            enrollmentToken = ""
        }
    }
}
