import AppKit
import CowboyInstallerCore
import SwiftUI

struct AccountView: View {
    @EnvironmentObject private var model: AppModel
    @State private var account = ""
    @State private var password = ""

    var body: some View {
        Form {
            Section("Cowboy Service") {
                TextField("Service URL", text: controllerBinding)
                    .textContentType(.URL)
                HStack {
                    Button("Refresh Status") { model.refreshAll() }
                    if model.controllerURL != nil {
                        Button("Open Cowboy") { CowboyOpener.open(controllerURL: model.controllerURL) }
                    }
                }
            }

            Section("Account") {
                Label(model.accountStatus.message, systemImage: accountSymbol)
                    .foregroundStyle(accountColor)
                if let name = model.accountStatus.account {
                    LabeledContent("Account", value: name)
                }
                if let role = model.accountStatus.role {
                    LabeledContent("Role", value: role.capitalized)
                }
                LabeledContent(
                    "Dependency controls",
                    value: model.accountStatus.canManageDependencies ? "Unlocked" : "Locked"
                )

                if model.accountStatus.phase == .checking {
                    ProgressView()
                        .controlSize(.small)
                }

                if needsCredentials {
                    TextField("Account", text: $account)
                        .textContentType(.username)
                    SecureField("Password", text: $password)
                        .textContentType(.password)
                    Button("Sign In") {
                        if model.signIn(account: account, password: password) {
                            password = ""
                        }
                    }
                    .keyboardShortcut(.defaultAction)
                    .disabled(account.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || password.isEmpty)
                } else if model.accountStatus.phase == .signedIn || model.accountStatus.administratorAccess {
                    Button("Sign Out") { model.signOut() }
                }

                if model.accountStatus.phase == .setupRequired {
                    Text("First-time account creation remains in Cowboy's browser UI so the host setup code never enters this app.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                if let error = model.accountStatus.errorMessage {
                    Text(error)
                        .foregroundStyle(.red)
                        .textSelection(.enabled)
                }
            }

            Section("Privacy") {
                Text("The password is sent only to the configured HTTPS Cowboy Service and is never persisted. macOS stores the resulting Service cookies for this app's URL session; the default browser keeps its own separate Cowboy session.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
        .padding(12)
        .navigationTitle("Account")
        .onAppear {
            if account.isEmpty, let current = model.accountStatus.account, current != "local" {
                account = current
            }
        }
    }

    private var needsCredentials: Bool {
        switch model.accountStatus.phase {
        case .signedOut, .failed:
            true
        case .localOwner, .signedIn:
            !model.accountStatus.administratorAccess
        default:
            false
        }
    }

    private var accountSymbol: String {
        switch model.accountStatus.phase {
        case .localOwner, .signedIn: "person.crop.circle.badge.checkmark"
        case .checking: "arrow.triangle.2.circlepath"
        case .setupRequired: "person.crop.circle.badge.plus"
        case .failed: "exclamationmark.triangle"
        default: "person.crop.circle"
        }
    }

    private var accountColor: Color {
        switch model.accountStatus.phase {
        case .localOwner, .signedIn: .green
        case .failed: .red
        case .setupRequired: .orange
        default: .secondary
        }
    }

    private var controllerBinding: Binding<String> {
        Binding(
            get: { model.settings.controllerURL },
            set: { value in model.updateSettings { $0.controllerURL = value } }
        )
    }
}
