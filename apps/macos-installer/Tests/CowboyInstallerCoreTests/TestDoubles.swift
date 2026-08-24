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
    notifier: StubNotifier = StubNotifier()
) -> AppModel {
    AppModel(
        backend: backend,
        persistence: persistence,
        statusDetector: detector,
        notifier: notifier,
        targetVersion: "1.2.3"
    )
}
