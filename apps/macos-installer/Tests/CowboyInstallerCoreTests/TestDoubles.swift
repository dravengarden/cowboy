import CowboyInstallerCore
import Foundation

@MainActor
final class MockInstallerBackend: InstallerBackend {
    enum Behavior {
        case succeed
        case fail(Error)
        case wait
    }

    var behavior: Behavior = .succeed
    private(set) var installCount = 0
    private var continuation: CheckedContinuation<Void, Never>?
    private var cancelled = false

    func install(
        request: InstallRequest,
        onProgress: @escaping (InstallProgress) -> Void
    ) async throws -> InstallReceipt {
        installCount += 1
        onProgress(.init(phase: .installing, fraction: 0.5, message: "Installing"))
        switch behavior {
        case .succeed:
            return receipt(for: request)
        case let .fail(error):
            throw error
        case .wait:
            await withCheckedContinuation { continuation = $0 }
            if cancelled { throw CancellationError() }
            return receipt(for: request)
        }
    }

    func cancel() {
        cancelled = true
        continuation?.resume()
        continuation = nil
    }

    func succeedWaitingInstall() {
        continuation?.resume()
        continuation = nil
    }

    private func receipt(for request: InstallRequest) -> InstallReceipt {
        InstallReceipt(
            installedVersion: request.targetVersion,
            installationDirectory: request.stateDirectory ?? "/state",
            log: "installed"
        )
    }
}

final class MemoryPersistence: InstallerPersistence {
    var settings = InstallerSettings()
    var history: [ActivityRecord] = []
    var pending: PendingInstall?

    func loadSettings() -> InstallerSettings { settings }
    func saveSettings(_ settings: InstallerSettings) { self.settings = settings }
    func loadHistory() -> [ActivityRecord] { history }
    func saveHistory(_ history: [ActivityRecord]) { self.history = history }
    func loadPendingInstall() -> PendingInstall? { pending }
    func savePendingInstall(_ pending: PendingInstall?) { self.pending = pending }
}

final class StubStatusDetector: InstalledStatusDetecting {
    var status: InstalledStatus = .notInstalled
    func detect(preferredStateDirectory _: String?) -> InstalledStatus { status }
}

final class MockMachineServiceController: MachineServiceControlling, @unchecked Sendable {
    var startError: Error?
    var stopError: Error?
    var onStart: (() -> Void)?
    var onStop: (() -> Void)?
    private(set) var startCount = 0
    private(set) var stopCount = 0

    func start(_: InstalledStatus) async throws {
        startCount += 1
        if let startError { throw startError }
        onStart?()
    }

    func stop(_: InstalledStatus) async throws {
        stopCount += 1
        if let stopError { throw stopError }
        onStop?()
    }
}

final class MockCowboyServiceClient: CowboyServiceClient, @unchecked Sendable {
    var status = AccountStatus(
        phase: .localOwner,
        account: "local",
        role: "owner",
        administratorAccess: true,
        message: "ready"
    )
    var signInStatus: AccountStatus?
    var machineSummary = ManagedMachineSummary(
        id: "macbook-air",
        displayName: "MacBook Air",
        status: "online",
        connected: true,
        activeSessions: 0,
        pendingUpdates: [],
        components: []
    )
    var plan = DependencyUpdatePlan(
        machineID: "macbook-air",
        machineName: "MacBook Air",
        activeSessions: 0,
        items: []
    )
    var error: Error?
    private(set) var signInCount = 0
    private(set) var signOutCount = 0
    private(set) var checkCount = 0
    private(set) var applied: [DependencyUpdateItem] = []

    func accountStatus(controllerURL _: String) async throws -> AccountStatus {
        if let error { throw error }
        return status
    }

    func signIn(controllerURL _: String, account _: String, password _: String) async throws -> AccountStatus {
        signInCount += 1
        if let error { throw error }
        return signInStatus ?? status
    }

    func signOut(controllerURL _: String) async throws -> AccountStatus {
        signOutCount += 1
        if let error { throw error }
        return AccountStatus(phase: .signedOut, message: "signed out")
    }

    func machine(controllerURL _: String, machineID: String) async throws -> ManagedMachineSummary {
        if let error { throw error }
        guard machineSummary.id == machineID else {
            throw CowboyServiceClientError.machineNotFound(machineID)
        }
        return machineSummary
    }

    func dependencyUpdatePlan(
        controllerURL _: String,
        machineID _: String,
        refresh _: Bool
    ) async throws -> DependencyUpdatePlan {
        checkCount += 1
        if let error { throw error }
        return plan
    }

    func applyDependencyUpdate(
        controllerURL _: String,
        machineID _: String,
        item: DependencyUpdateItem
    ) async throws {
        if let error { throw error }
        applied.append(item)
    }
}

final class MemoryServiceCredentialStore: ServiceCredentialStoring {
    var credentials: [String: ServiceCredential] = [:]
    var error: Error?
    private(set) var saves: [(String, ServiceCredential)] = []
    private(set) var deletes: [String] = []

    func load(controllerURL: String) throws -> ServiceCredential? {
        if let error { throw error }
        return credentials[controllerURL]
    }

    func save(_ credential: ServiceCredential, controllerURL: String) throws {
        if let error { throw error }
        credentials[controllerURL] = credential
        saves.append((controllerURL, credential))
    }

    func delete(controllerURL: String) throws {
        if let error { throw error }
        credentials[controllerURL] = nil
        deletes.append(controllerURL)
    }
}

final class StubNotifier: InstallerNotifying {
    var authorization = true
    private(set) var notifications: [(String, String)] = []

    func requestAuthorization() async -> Bool { authorization }
    func send(title: String, body: String) async { notifications.append((title, body)) }
}

struct TestFailure: LocalizedError {
    let errorDescription: String?
}

func testRequest(token: String = "secret") -> InstallRequest {
    InstallRequest(
        controllerURL: "https://cowboy.example",
        enrollmentToken: token,
        workspaceDirectory: FileManager.default.homeDirectoryForCurrentUser.path,
        stateDirectory: "/state",
        targetVersion: "1.2.3"
    )
}

@MainActor
func makeModel(
    backend: MockInstallerBackend,
    persistence: MemoryPersistence = MemoryPersistence(),
    detector: StubStatusDetector = StubStatusDetector(),
    notifier: StubNotifier = StubNotifier(),
    machineService: MockMachineServiceController = MockMachineServiceController(),
    serviceClient: MockCowboyServiceClient = MockCowboyServiceClient(),
    credentialStore: MemoryServiceCredentialStore = MemoryServiceCredentialStore()
) -> AppModel {
    AppModel(
        backend: backend,
        persistence: persistence,
        statusDetector: detector,
        notifier: notifier,
        machineService: machineService,
        serviceClient: serviceClient,
        credentialStore: credentialStore,
        targetVersion: "1.2.3"
    )
}
