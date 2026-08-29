import AppKit
import CowboyInstallerCore
import SwiftUI

struct AccountView: View {
    @EnvironmentObject private var model: AppModel
    @State private var account = ""
    @State private var password = ""
    @State private var rememberLogin = true
    @State private var showsOptionalSignIn = false

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
                    value: dependencyControlsLabel
                )

                if model.accountStatus.phase == .checking {
                    ProgressView()
                        .controlSize(.small)
                }

                if optionalSignInAvailable && !showsOptionalSignIn {
                    Text("Account passwords and passkeys are optional for local access. Continue using Cowboy now and configure authentication later when you want protected administrator actions.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    Button("Use Account Password…") {
                        showsOptionalSignIn = true
                    }
                }

                if needsCredentials && (!optionalSignInAvailable || showsOptionalSignIn) {
                    TextField("Account", text: $account)
                        .textContentType(.username)
                    SecureField("Password", text: $password)
                        .textContentType(.password)
                    HStack {
                        Button("Sign In") {
                            if model.signIn(
                                account: account,
                                password: password,
                                remember: rememberLogin
                            ) {
                                password = ""
                            }
                        }
                        .keyboardShortcut(.defaultAction)
                        .disabled(account.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || password.isEmpty)
                        if optionalSignInAvailable {
                            Button("Not Now") {
                                password = ""
                                showsOptionalSignIn = false
                            }
                        }
                    }
                    Toggle(
                        "Save this optional sign-in in macOS Keychain",
                        isOn: $rememberLogin
                    )
                } else if model.accountStatus.phase == .signedIn || model.accountStatus.administratorAccess {
                    Button("Sign Out") { model.signOut() }
                }

                if model.savedLoginAvailable {
                    Label("Automatic sign-in is stored in macOS Keychain.", systemImage: "key.fill")
                        .foregroundStyle(.green)
                    Button("Forget Saved Login") {
                        model.forgetSavedLogin()
                        rememberLogin = false
                    }
                }
                if let error = model.credentialStorageError {
                    Text(error)
                        .foregroundStyle(.red)
                        .textSelection(.enabled)
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
                Text("Local-owner mode does not require an account password or passkey. If optional password sign-in is used, the password is sent only to the configured HTTPS Cowboy Service. When saving is selected, it is stored only in this app's macOS Keychain item; it never enters settings, logs, activity history, or the browser.")
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
            if model.savedLoginAvailable {
                rememberLogin = true
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

    private var optionalSignInAvailable: Bool {
        model.accountStatus.authenticationIsOptional
            && !model.accountStatus.administratorAccess
    }

    private var dependencyControlsLabel: String {
        if model.accountStatus.canManageDependencies {
            return "Unlocked"
        }
        return optionalSignInAvailable ? "Protected" : "Locked"
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
