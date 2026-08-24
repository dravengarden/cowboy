import Foundation

@MainActor
public final class AppModel: ObservableObject {
    @Published public private(set) var installState: InstallState
    @Published public private(set) var installedStatus: InstalledStatus
    @Published public private(set) var history: [ActivityRecord]
    @Published public private(set) var settings: InstallerSettings

    public let targetVersion: String

    private let backend: InstallerBackend
    private let persistence: InstallerPersistence
    private let statusDetector: InstalledStatusDetecting
    private let notifier: InstallerNotifying
    private var installationTask: Task<Void, Never>?
    private var automaticStatusTask: Task<Void, Never>?
    private var currentStartedAt: Date?
    private var lastRequestWithoutToken: InstallRequest?

    public init(
        backend: InstallerBackend,
        persistence: InstallerPersistence,
        statusDetector: InstalledStatusDetecting,
        notifier: InstallerNotifying,
        targetVersion: String
    ) {
        self.backend = backend
        self.persistence = persistence
        self.statusDetector = statusDetector
        self.notifier = notifier
        self.targetVersion = targetVersion
        settings = persistence.loadSettings()
        history = persistence.loadHistory()
        installedStatus = statusDetector.detect(
            preferredStateDirectory: settings.stateDirectory.nilIfEmpty
        )
        installState = InstallState()
        recoverInterruptedInstallIfNeeded()
        configureAutomaticStatusChecks()
    }

    public var isRunning: Bool { installState.phase.isRunning }

    public var canRetry: Bool {
        [.failed, .cancelled, .interrupted].contains(installState.phase)
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
                installedStatus = statusDetector.detect(preferredStateDirectory: request.stateDirectory)
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
    }

    public func updateSettings(_ update: (inout InstallerSettings) -> Void) {
        let previousAutomaticCheck = settings.automaticallyCheckForUpdates
        update(&settings)
        persistence.saveSettings(settings)
        if previousAutomaticCheck != settings.automaticallyCheckForUpdates {
            configureAutomaticStatusChecks()
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
        persistence.saveHistory(Array(history.prefix(50)))
        persistence.savePendingInstall(nil)
        installState = InstallState(
            phase: .interrupted,
            progress: 0,
            message: summary,
            errorMessage: "Create a new one-time enrollment code and retry."
        )
    }

    private func configureAutomaticStatusChecks() {
        automaticStatusTask?.cancel()
        automaticStatusTask = nil
        guard settings.automaticallyCheckForUpdates else { return }
        automaticStatusTask = Task { [weak self] in
            while !Task.isCancelled {
                do {
                    try await Task.sleep(for: .seconds(1_800))
                } catch {
                    return
                }
                self?.refreshInstalledStatus()
            }
        }
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
        return AppRuntime(
            model: AppModel(
                backend: ProcessInstallerBackend(),
                persistence: UserDefaultsInstallerPersistence(),
                statusDetector: InstalledStatusDetector(),
                notifier: SystemInstallerNotifier(),
                targetVersion: version
            ),
            launchAtLogin: LaunchAtLoginController()
        )
    }
}

private extension String {
    var nilIfEmpty: String? { isEmpty ? nil : self }
}
