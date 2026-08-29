import Foundation

@MainActor
public final class AppModel: ObservableObject {
    @Published public private(set) var installState: InstallState
    @Published public private(set) var installedStatus: InstalledStatus
    @Published public private(set) var history: [ActivityRecord]
    @Published public private(set) var settings: InstallerSettings
    @Published public private(set) var serviceActionState: MachineServiceActionState
    @Published public private(set) var accountStatus: AccountStatus
    @Published public private(set) var remoteMachine: ManagedMachineSummary?
    @Published public private(set) var remoteStatusError: String?
    @Published public private(set) var dependencyUpdateState: DependencyUpdateState
    @Published public private(set) var savedLoginAvailable: Bool
    @Published public private(set) var credentialStorageError: String?

    public let targetVersion: String

    private let backend: InstallerBackend
    private let persistence: InstallerPersistence
    private let statusDetector: InstalledStatusDetecting
    private let notifier: InstallerNotifying
    private let machineService: MachineServiceControlling
    private let serviceClient: CowboyServiceClient
    private let credentialStore: ServiceCredentialStoring
    private var installationTask: Task<Void, Never>?
    private var serviceActionTask: Task<Void, Never>?
    private var accountTask: Task<Void, Never>?
    private var dependencyTask: Task<Void, Never>?
    private var monitoringTask: Task<Void, Never>?
    private var currentStartedAt: Date?
    private var lastRequestWithoutToken: InstallRequest?
    private var rejectedAutomaticCredential: ServiceCredential?
    private var nextAutomaticSignInAt = Date.distantPast

    public init(
        backend: InstallerBackend,
        persistence: InstallerPersistence,
        statusDetector: InstalledStatusDetecting,
        notifier: InstallerNotifying,
        machineService: MachineServiceControlling = LaunchctlMachineServiceController(),
        serviceClient: CowboyServiceClient = URLSessionCowboyServiceClient(),
        credentialStore: ServiceCredentialStoring = KeychainServiceCredentialStore(),
        targetVersion: String
    ) {
        self.backend = backend
        self.persistence = persistence
        self.statusDetector = statusDetector
        self.notifier = notifier
        self.machineService = machineService
        self.serviceClient = serviceClient
        self.credentialStore = credentialStore
        self.targetVersion = targetVersion

        var loadedSettings = persistence.loadSettings()
        let detectedStatus = statusDetector.detect(
            preferredStateDirectory: loadedSettings.stateDirectory.nilIfEmpty
        )
        if loadedSettings.controllerURL.isEmpty, let origin = detectedStatus.serviceOrigin {
            loadedSettings.controllerURL = origin
            persistence.saveSettings(loadedSettings)
        }
        settings = loadedSettings
        history = persistence.loadHistory()
        installedStatus = detectedStatus
        installState = InstallState()
        serviceActionState = MachineServiceActionState()
        accountStatus = AccountStatus()
        remoteMachine = nil
        remoteStatusError = nil
        dependencyUpdateState = DependencyUpdateState()
        savedLoginAvailable = false
        credentialStorageError = nil
        refreshSavedLoginAvailability()
        recoverInterruptedInstallIfNeeded()
    }

    public var isRunning: Bool { installState.phase.isRunning }

    public var isMachineRunning: Bool { installedStatus.launchAgentLoaded }

    public var isBusy: Bool {
        isRunning || serviceActionState.phase.isRunning || dependencyUpdateState.phase.isRunning
    }

    public var canRetry: Bool {
        [.failed, .cancelled, .interrupted].contains(installState.phase)
    }

    public var controllerURL: URL? {
        let value = settings.controllerURL.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let url = URL(string: value), url.host != nil else { return nil }
        return url
    }

    public func startMonitoring() {
        guard monitoringTask == nil else { return }
        monitoringTask = Task { [weak self] in
            guard let self else { return }
            await refreshAllAndWait(checkDependencies: settings.automaticallyCheckForUpdates)
            while !Task.isCancelled {
                do {
                    try await Task.sleep(for: .seconds(30))
                } catch {
                    return
                }
                await refreshAllAndWait(checkDependencies: settings.automaticallyCheckForUpdates)
            }
        }
    }

    @discardableResult
    public func startInstall(_ request: InstallRequest) -> Bool {
        guard installationTask == nil else { return false }
        let startedAt = Date()
        currentStartedAt = startedAt
        lastRequestWithoutToken = InstallRequest(
            controllerURL: request.controllerURL,
            enrollmentToken: "",
            workspaceDirectory: request.workspaceDirectory,
            stateDirectory: request.stateDirectory,
            targetVersion: request.targetVersion
        )
        persistence.savePendingInstall(.init(startedAt: startedAt, targetVersion: request.targetVersion))
        apply(.init(phase: .preparing, fraction: 0.02, message: "Preparing installation"))

        installationTask = Task { [weak self] in
            guard let self else { return }
            do {
                let receipt = try await backend.install(request: request) { [weak self] progress in
                    self?.apply(progress)
                }
                apply(.init(phase: .refreshing, fraction: 0.94, message: "Refreshing installed status"))
                refreshInstalledStatus()
                if !installedStatus.isInstalled {
                    installedStatus = InstalledStatus(
                        isInstalled: true,
                        version: receipt.installedVersion,
                        location: receipt.installationDirectory,
                        serviceOrigin: request.controllerURL,
                        launchAgentLoaded: true
                    )
                }
                finish(
                    result: .succeeded,
                    phase: .succeeded,
                    summary: "Cowboy Machine \(receipt.installedVersion) installed",
                    details: receipt.log,
                    targetVersion: request.targetVersion
                )
                await refreshRemoteState(checkDependencies: false)
                if settings.notificationsEnabled {
                    await notifier.send(title: "Cowboy is ready", body: "Cowboy Machine was installed successfully.")
                }
            } catch is CancellationError {
                finish(
                    result: .cancelled,
                    phase: .cancelled,
                    summary: "Installation cancelled",
                    details: "The installation was cancelled before completion.",
                    targetVersion: request.targetVersion
                )
            } catch {
                let message = Self.userFacing(error)
                finish(
                    result: .failed,
                    phase: .failed,
                    summary: "Installation failed",
                    details: message,
                    targetVersion: request.targetVersion,
                    errorMessage: message
                )
                if settings.notificationsEnabled {
                    await notifier.send(title: "Cowboy needs attention", body: message)
                }
            }
        }
        return true
    }

    public func cancelInstall() {
        guard installationTask != nil else { return }
        backend.cancel()
        installationTask?.cancel()
    }

    public func waitForCurrentInstall() async {
        await installationTask?.value
    }

    public func refreshInstalledStatus() {
        installedStatus = statusDetector.detect(
            preferredStateDirectory: settings.stateDirectory.nilIfEmpty
        )
        if settings.controllerURL.isEmpty, let origin = installedStatus.serviceOrigin {
            settings.controllerURL = origin
            persistence.saveSettings(settings)
        }
    }

    public func refreshAll() {
        Task { [weak self] in
            await self?.refreshAllAndWait()
        }
    }

    public func refreshAllAndWait(checkDependencies: Bool = false) async {
        refreshInstalledStatus()
        await refreshRemoteState(checkDependencies: checkDependencies)
    }

    @discardableResult
    public func startMachine() -> Bool {
        guard serviceActionTask == nil, installedStatus.isInstalled, !isMachineRunning else {
            return false
        }
        let status = installedStatus
        serviceActionState = MachineServiceActionState(
            phase: .starting,
            message: "Starting Cowboy Machine"
        )
        serviceActionTask = Task { [weak self] in
            guard let self else { return }
            do {
                try await machineService.start(status)
                try await refreshMachineService(untilRunning: true)
                serviceActionState = MachineServiceActionState(message: "Cowboy Machine is running")
                await refreshRemoteState(checkDependencies: false)
            } catch {
                let message = Self.userFacing(error)
                serviceActionState = MachineServiceActionState(
                    phase: .failed,
                    message: "Could not start Cowboy Machine",
                    errorMessage: message
                )
            }
            serviceActionTask = nil
        }
        return true
    }

    @discardableResult
    public func stopMachine() -> Bool {
        guard serviceActionTask == nil, installedStatus.isInstalled, isMachineRunning else {
            return false
        }
        let status = installedStatus
        serviceActionState = MachineServiceActionState(
            phase: .stopping,
            message: "Stopping Cowboy Machine"
        )
        serviceActionTask = Task { [weak self] in
            guard let self else { return }
            do {
                try await machineService.stop(status)
                try await refreshMachineService(untilRunning: false)
                serviceActionState = MachineServiceActionState(message: "Cowboy Machine is stopped")
                await refreshRemoteState(checkDependencies: false)
            } catch {
                let message = Self.userFacing(error)
                serviceActionState = MachineServiceActionState(
                    phase: .failed,
                    message: "Could not stop Cowboy Machine",
                    errorMessage: message
                )
            }
            serviceActionTask = nil
        }
        return true
    }

    public func waitForServiceAction() async {
        await serviceActionTask?.value
    }

    @discardableResult
    public func signIn(account: String, password: String, remember: Bool = true) -> Bool {
        rejectedAutomaticCredential = nil
        nextAutomaticSignInAt = .distantPast
        return beginSignIn(
            credential: ServiceCredential(account: account, password: password),
            remember: remember,
            automatic: false
        )
    }

    @discardableResult
    public func signOut() -> Bool {
        guard accountTask == nil, !settings.controllerURL.isEmpty else { return false }
        forgetSavedLogin()
        accountStatus = AccountStatus(phase: .checking, message: "Signing out")
        accountTask = Task { [weak self] in
            guard let self else { return }
            do {
                accountStatus = try await serviceClient.signOut(controllerURL: settings.controllerURL)
                remoteMachine = nil
                remoteStatusError = nil
            } catch {
                accountStatus = AccountStatus(
                    phase: .failed,
                    message: "Sign-out failed",
                    errorMessage: Self.userFacing(error)
                )
            }
            accountTask = nil
        }
        return true
    }

    public func waitForAccountAction() async {
        await accountTask?.value
    }

    public func forgetSavedLogin() {
        do {
            try credentialStore.delete(controllerURL: settings.controllerURL)
            savedLoginAvailable = false
            credentialStorageError = nil
            rejectedAutomaticCredential = nil
            nextAutomaticSignInAt = .distantPast
        } catch {
            credentialStorageError = Self.userFacing(error)
        }
    }

    @discardableResult
    public func checkDependencies(refresh: Bool = true) -> Bool {
        guard dependencyTask == nil,
              accountStatus.canReadProduct,
              let machineID = installedStatus.machineID,
              !settings.controllerURL.isEmpty
        else {
            return false
        }
        dependencyUpdateState = DependencyUpdateState(
            phase: .checking,
            progress: 0.05,
            message: refresh ? "Checking dependency releases" : "Refreshing dependency status"
        )
        dependencyTask = Task { [weak self] in
            guard let self else { return }
            do {
                let plan = try await serviceClient.dependencyUpdatePlan(
                    controllerURL: settings.controllerURL,
                    machineID: machineID,
                    refresh: refresh
                )
                dependencyUpdateState = DependencyUpdateState(
                    phase: plan.items.isEmpty ? .succeeded : .ready,
                    progress: 1,
                    message: plan.items.isEmpty
                        ? "All managed dependencies are up to date"
                        : "\(plan.items.count) dependency update\(plan.items.count == 1 ? "" : "s") available",
                    plan: plan
                )
                remoteMachine = try? await serviceClient.machine(
                    controllerURL: settings.controllerURL,
                    machineID: machineID
                )
            } catch {
                dependencyUpdateState = DependencyUpdateState(
                    phase: .failed,
                    message: "Dependency check failed",
                    errorMessage: Self.userFacing(error)
                )
            }
            dependencyTask = nil
        }
        return true
    }

    @discardableResult
    public func applyDependencyUpdates() -> Bool {
        guard dependencyTask == nil,
              accountStatus.canManageDependencies,
              let plan = dependencyUpdateState.plan,
              !plan.items.isEmpty,
              !settings.controllerURL.isEmpty
        else {
            return false
        }
        dependencyUpdateState = DependencyUpdateState(
            phase: .updating,
            progress: 0,
            message: "Preparing dependency updates",
            plan: plan
        )
        dependencyTask = Task { [weak self] in
            guard let self else { return }
            do {
                for (index, item) in plan.items.enumerated() {
                    dependencyUpdateState = DependencyUpdateState(
                        phase: .updating,
                        progress: Double(index) / Double(plan.items.count),
                        message: "Updating \(item.displayName)",
                        plan: plan
                    )
                    try await serviceClient.applyDependencyUpdate(
                        controllerURL: settings.controllerURL,
                        machineID: plan.machineID,
                        item: item
                    )
                }
                let remaining = try await serviceClient.dependencyUpdatePlan(
                    controllerURL: settings.controllerURL,
                    machineID: plan.machineID,
                    refresh: false
                )
                dependencyUpdateState = DependencyUpdateState(
                    phase: .succeeded,
                    progress: 1,
                    message: remaining.items.isEmpty
                        ? "Dependencies updated successfully"
                        : "Updates completed; \(remaining.items.count) item\(remaining.items.count == 1 ? "" : "s") still need attention",
                    plan: remaining
                )
                remoteMachine = try? await serviceClient.machine(
                    controllerURL: settings.controllerURL,
                    machineID: plan.machineID
                )
                if settings.notificationsEnabled {
                    await notifier.send(
                        title: "Cowboy dependencies updated",
                        body: dependencyUpdateState.message
                    )
                }
            } catch {
                let message = Self.userFacing(error)
                dependencyUpdateState = DependencyUpdateState(
                    phase: .failed,
                    progress: dependencyUpdateState.progress,
                    message: "Dependency update failed",
                    plan: plan,
                    errorMessage: message
                )
                if settings.notificationsEnabled {
                    await notifier.send(title: "Cowboy update needs attention", body: message)
                }
            }
            dependencyTask = nil
        }
        return true
    }

    public func waitForDependencyAction() async {
        await dependencyTask?.value
    }

    public func updateSettings(_ update: (inout InstallerSettings) -> Void) {
        let previousControllerURL = settings.controllerURL
        update(&settings)
        persistence.saveSettings(settings)
        if settings.controllerURL != previousControllerURL {
            rejectedAutomaticCredential = nil
            nextAutomaticSignInAt = .distantPast
            refreshSavedLoginAvailability()
        }
    }

    public func requestNotificationAuthorization() async -> Bool {
        await notifier.requestAuthorization()
    }

    public func windowDidClose() {
        // Intentionally empty: AppModel is owned by the App scene, not a window.
    }

    public func retryTemplate() -> InstallRequest? {
        lastRequestWithoutToken
    }

    private func refreshMachineService(untilRunning expected: Bool) async throws {
        for _ in 0..<20 {
            refreshInstalledStatus()
            if installedStatus.launchAgentLoaded == expected {
                return
            }
            try await Task.sleep(for: .milliseconds(250))
        }
        throw MachineServiceControllerError.commandFailed(
            expected
                ? "Cowboy Machine was loaded but did not become ready."
                : "Cowboy Machine did not stop before the timeout."
        )
    }

    private func refreshRemoteState(checkDependencies: Bool) async {
        guard accountTask == nil else { return }
        guard !settings.controllerURL.isEmpty else {
            accountStatus = AccountStatus(
                phase: .unknown,
                message: "Set the Cowboy Service URL to check account status."
            )
            remoteMachine = nil
            return
        }
        do {
            accountStatus = try await serviceClient.accountStatus(controllerURL: settings.controllerURL)
            await attemptAutomaticSignInIfNeeded()
            await refreshRemoteMachine()
            if checkDependencies,
               dependencyTask == nil,
               accountStatus.canReadProduct,
               installedStatus.machineID != nil
            {
                _ = self.checkDependencies(refresh: false)
            }
        } catch {
            accountStatus = AccountStatus(
                phase: .failed,
                message: "Could not reach Cowboy Service",
                errorMessage: Self.userFacing(error)
            )
            remoteMachine = nil
            remoteStatusError = Self.userFacing(error)
        }
    }

    private func refreshRemoteMachine() async {
        guard accountStatus.canReadProduct,
              let machineID = installedStatus.machineID,
              !settings.controllerURL.isEmpty
        else {
            remoteMachine = nil
            remoteStatusError = nil
            return
        }
        do {
            remoteMachine = try await serviceClient.machine(
                controllerURL: settings.controllerURL,
                machineID: machineID
            )
            remoteStatusError = nil
        } catch {
            remoteMachine = nil
            remoteStatusError = Self.userFacing(error)
        }
    }

    private func beginSignIn(
        credential: ServiceCredential,
        remember: Bool,
        automatic: Bool
    ) -> Bool {
        guard accountTask == nil, !settings.controllerURL.isEmpty else { return false }
        let previousStatus = accountStatus
        accountStatus = AccountStatus(
            phase: .checking,
            message: automatic ? "Restoring all Cowboy accounts" : "Signing in"
        )
        accountTask = Task { [weak self] in
            guard let self else { return }
            do {
                accountStatus = try await serviceClient.signIn(
                    controllerURL: settings.controllerURL,
                    account: credential.account,
                    password: credential.password
                )
                if remember {
                    saveCredential(credential)
                } else {
                    forgetSavedLogin()
                }
                rejectedAutomaticCredential = nil
                nextAutomaticSignInAt = .distantPast
                await refreshRemoteMachine()
            } catch {
                if automatic {
                    if Self.isCredentialRejection(error) {
                        rejectedAutomaticCredential = credential
                    } else {
                        nextAutomaticSignInAt = Date().addingTimeInterval(60)
                    }
                }
                accountStatus = Self.signInFailureStatus(
                    preserving: previousStatus,
                    automatic: automatic,
                    error: error
                )
            }
            accountTask = nil
        }
        return true
    }

    private func attemptAutomaticSignInIfNeeded() async {
        guard accountTask == nil,
              !accountStatus.canManageDependencies,
              Date() >= nextAutomaticSignInAt
        else {
            return
        }
        let credential: ServiceCredential
        do {
            guard let loaded = try credentialStore.load(controllerURL: settings.controllerURL) else {
                savedLoginAvailable = false
                credentialStorageError = nil
                return
            }
            credential = loaded
            savedLoginAvailable = true
            credentialStorageError = nil
        } catch {
            savedLoginAvailable = false
            credentialStorageError = Self.userFacing(error)
            return
        }
        guard credential != rejectedAutomaticCredential else { return }
        if beginSignIn(credential: credential, remember: true, automatic: true) {
            await accountTask?.value
        }
    }

    private func saveCredential(_ credential: ServiceCredential) {
        do {
            try credentialStore.save(credential, controllerURL: settings.controllerURL)
            savedLoginAvailable = true
            credentialStorageError = nil
        } catch {
            savedLoginAvailable = false
            credentialStorageError = "Signed in, but automatic sign-in was not saved. \(Self.userFacing(error))"
        }
    }

    private func refreshSavedLoginAvailability() {
        guard !settings.controllerURL.isEmpty else {
            savedLoginAvailable = false
            credentialStorageError = nil
            return
        }
        do {
            savedLoginAvailable = try credentialStore.load(controllerURL: settings.controllerURL) != nil
            credentialStorageError = nil
        } catch {
            savedLoginAvailable = false
            credentialStorageError = Self.userFacing(error)
        }
    }

    private static func signInFailureStatus(
        preserving previous: AccountStatus,
        automatic: Bool,
        error: Error
    ) -> AccountStatus {
        if previous.canReadProduct {
            var preserved = previous
            preserved.message = automatic
                ? "Local access remains available; automatic administrator sign-in failed."
                : "Local access remains available; administrator sign-in failed."
            preserved.errorMessage = userFacing(error)
            return preserved
        }
        return AccountStatus(
            phase: .failed,
            message: automatic ? "Automatic sign-in failed" : "Sign-in failed",
            errorMessage: userFacing(error)
        )
    }

    private static func isCredentialRejection(_ error: Error) -> Bool {
        guard case let CowboyServiceClientError.requestFailed(status, _) = error else {
            return false
        }
        return status == 400 || status == 401 || status == 403
    }

    private func apply(_ progress: InstallProgress) {
        installState = InstallState(
            phase: progress.phase,
            progress: min(max(progress.fraction, 0), 1),
            message: progress.message
        )
    }

    private func finish(
        result: ActivityResult,
        phase: InstallPhase,
        summary: String,
        details: String,
        targetVersion: String,
        errorMessage: String? = nil
    ) {
        let startedAt = currentStartedAt ?? Date()
        let record = ActivityRecord(
            startedAt: startedAt,
            endedAt: Date(),
            targetVersion: targetVersion,
            result: result,
            summary: summary,
            details: details
        )
        history.insert(record, at: 0)
        history = Array(history.prefix(50))
        persistence.saveHistory(history)
        persistence.savePendingInstall(nil)
        installState = InstallState(
            phase: phase,
            progress: phase == .succeeded ? 1 : installState.progress,
            message: summary,
            errorMessage: errorMessage
        )
        installationTask = nil
        currentStartedAt = nil
    }

    private func recoverInterruptedInstallIfNeeded() {
        guard let pending = persistence.loadPendingInstall() else { return }
        let summary = "Previous installation was interrupted"
        history.insert(
            ActivityRecord(
                startedAt: pending.startedAt,
                endedAt: Date(),
                targetVersion: pending.targetVersion,
                result: .interrupted,
                summary: summary,
                details: "Cowboy cannot safely resume the enrollment command. Create a new one-time code and retry."
            ),
            at: 0
        )
        history = Array(history.prefix(50))
        persistence.saveHistory(history)
        persistence.savePendingInstall(nil)
        installState = InstallState(
            phase: .interrupted,
            progress: 0,
            message: summary,
            errorMessage: "Create a new one-time enrollment code and retry."
        )
    }

    private static func userFacing(_ error: Error) -> String {
        if let localized = error as? LocalizedError, let description = localized.errorDescription {
            return description
        }
        return error.localizedDescription
    }
}

@MainActor
public final class AppRuntime {
    public let model: AppModel
    public let launchAtLogin: LaunchAtLoginController

    public init(model: AppModel, launchAtLogin: LaunchAtLoginController) {
        self.model = model
        self.launchAtLogin = launchAtLogin
    }

    public var menuModel: AppModel { model }
    public var windowModel: AppModel { model }

    public static func live() -> AppRuntime {
        let version = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "development"
        let model = AppModel(
            backend: ProcessInstallerBackend(),
            persistence: UserDefaultsInstallerPersistence(),
            statusDetector: InstalledStatusDetector(),
            notifier: SystemInstallerNotifier(),
            machineService: LaunchctlMachineServiceController(),
            serviceClient: URLSessionCowboyServiceClient(),
            credentialStore: KeychainServiceCredentialStore(),
            targetVersion: version
        )
        return AppRuntime(
            model: model,
            launchAtLogin: LaunchAtLoginController()
        )
    }
}

private extension String {
    var nilIfEmpty: String? { isEmpty ? nil : self }
}
