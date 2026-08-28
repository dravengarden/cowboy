import AppKit
import CowboyInstallerCore
import SwiftUI

struct DashboardView: View {
    @EnvironmentObject private var model: AppModel
    @State private var showStopConfirmation = false
    @State private var showUpdateConfirmation = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                header
                machineCard
                dependencyCard
            }
            .padding(24)
            .frame(maxWidth: 720, alignment: .leading)
        }
        .navigationTitle("Dashboard")
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
        .confirmationDialog(
            "Update managed dependencies?",
            isPresented: $showUpdateConfirmation,
            titleVisibility: .visible
        ) {
            Button("Update All") { model.applyDependencyUpdates() }
            Button("Cancel", role: .cancel) {}
        } message: {
            if let plan = model.dependencyUpdateState.plan, plan.requiresConfirmation {
                Text("Active sessions use one or more affected dependencies. Cowboy will finish current turns and roll workers gradually; sessions may reconnect briefly.")
            } else {
                Text("Cowboy will apply each update through the existing Machine control channel.")
            }
        }
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 5) {
            Text("Cowboy")
                .font(.largeTitle.weight(.semibold))
            Text("A native, lightweight control surface for this Mac's Cowboy Machine.")
                .foregroundStyle(.secondary)
        }
    }

    private var machineCard: some View {
        GroupBox("Machine") {
            VStack(alignment: .leading, spacing: 14) {
                HStack(spacing: 10) {
                    Image(systemName: model.menuBarSymbol)
                        .font(.title2)
                        .foregroundStyle(model.menuBarColor)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(model.menuBarMessage)
                            .font(.headline)
                        if let remote = model.remoteMachine {
                            Text("\(remote.displayName) · \(remote.status) · \(remote.activeSessions) active session\(remote.activeSessions == 1 ? "" : "s")")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
                    Spacer()
                }

                if model.installedStatus.isInstalled {
                    LabeledContent("Machine ID", value: model.installedStatus.machineID ?? "Unknown")
                    LabeledContent("Version", value: model.installedStatus.version ?? "Unknown")
                    LabeledContent("State", value: model.isMachineRunning ? "Running in background" : "Stopped")
                    if let origin = model.installedStatus.serviceOrigin ?? model.controllerURL?.absoluteString {
                        LabeledContent("Cowboy Service", value: origin)
                    }
                } else {
                    Text("Install and enroll Cowboy Machine from the Install section before starting it.")
                        .foregroundStyle(.secondary)
                }

                if let error = model.serviceActionState.errorMessage ?? model.remoteStatusError {
                    Text(error)
                        .foregroundStyle(.red)
                        .textSelection(.enabled)
                }

                HStack {
                    Button("Refresh") { model.refreshAll() }
                    if model.controllerURL != nil {
                        Button("Open Cowboy") { CowboyOpener.open(controllerURL: model.controllerURL) }
                            .keyboardShortcut("o")
                    }
                    Spacer()
                    if model.installedStatus.isInstalled {
                        if model.isMachineRunning {
                            Button("Stop…", role: .destructive) { showStopConfirmation = true }
                                .disabled(model.serviceActionState.phase.isRunning)
                        } else {
                            Button("Start") { model.startMachine() }
                                .keyboardShortcut(.defaultAction)
                                .disabled(model.serviceActionState.phase.isRunning)
                        }
                    }
                }
            }
            .padding(8)
        }
    }

    private var dependencyCard: some View {
        GroupBox("Dependencies") {
            VStack(alignment: .leading, spacing: 12) {
                HStack {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(model.dependencyUpdateState.message)
                            .font(.headline)
                        Text(model.accountStatus.canManageDependencies
                            ? "Updates use the authenticated Cowboy Machine control channel."
                            : "Sign in with the owner account to unlock dependency updates.")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    Button("Check for Updates") { model.checkDependencies() }
                        .disabled(
                            !model.accountStatus.canReadProduct
                                || model.installedStatus.machineID == nil
                                || model.dependencyUpdateState.phase.isRunning
                        )
                }

                if model.dependencyUpdateState.phase.isRunning {
                    ProgressView(value: model.dependencyUpdateState.progress)
                }

                if let plan = model.dependencyUpdateState.plan, !plan.items.isEmpty {
                    Divider()
                    ForEach(plan.items) { item in
                        HStack(alignment: .firstTextBaseline) {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(item.displayName)
                                Text("\(item.currentVersion.isEmpty ? "Not installed" : item.currentVersion) → \(item.targetVersion)")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                            Spacer()
                            if item.activeLeases > 0 {
                                Text("\(item.activeLeases) active")
                                    .font(.caption)
                                    .foregroundStyle(.orange)
                            }
                        }
                    }
                    HStack {
                        if plan.requiresConfirmation {
                            Label("Active sessions will be rolled gradually", systemImage: "exclamationmark.triangle")
                                .font(.caption)
                                .foregroundStyle(.orange)
                        }
                        Spacer()
                        Button("Update All") { showUpdateConfirmation = true }
                            .buttonStyle(.borderedProminent)
                            .disabled(!model.accountStatus.canManageDependencies)
                    }
                }

                if let error = model.dependencyUpdateState.errorMessage {
                    Text(error)
                        .foregroundStyle(.red)
                        .textSelection(.enabled)
                }
            }
            .padding(8)
        }
    }
}
