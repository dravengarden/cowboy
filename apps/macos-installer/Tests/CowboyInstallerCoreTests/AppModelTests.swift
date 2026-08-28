import CowboyInstallerCore
import Foundation
import Testing

@MainActor
struct AppModelTests {
    @Test
    func rejectsConcurrentInstall() async {
        let backend = MockInstallerBackend()
        backend.behavior = .wait
        let model = makeModel(backend: backend)

        #expect(model.startInstall(testRequest()))
        #expect(!model.startInstall(testRequest(token: "another-secret")))
        await Task.yield()
        #expect(backend.installCount == 1)

        backend.succeedWaitingInstall()
        await model.waitForCurrentInstall()
        #expect(model.installState.phase == .succeeded)
    }

    @Test
    func windowCloseDoesNotCancelBackgroundTask() async {
        let backend = MockInstallerBackend()
        backend.behavior = .wait
        let model = makeModel(backend: backend)
        let runtime = AppRuntime(model: model, launchAtLogin: LaunchAtLoginController(service: MockLoginItemService()))

        #expect(model.startInstall(testRequest()))
        await Task.yield()
        model.windowDidClose()

        #expect(runtime.model === model)
        #expect(model.isRunning)
        #expect(backend.installCount == 1)

        backend.succeedWaitingInstall()
        await model.waitForCurrentInstall()
    }

    @Test
    func recordsSuccessAndFailureTransitions() async {
        let success = MockInstallerBackend()
        let successModel = makeModel(backend: success)
        #expect(successModel.startInstall(testRequest()))
        await successModel.waitForCurrentInstall()
        #expect(successModel.installState.phase == .succeeded)
        #expect(successModel.history.first?.result == .succeeded)

        let failure = MockInstallerBackend()
        failure.behavior = .fail(TestFailure(errorDescription: "network unavailable"))
        let failureModel = makeModel(backend: failure)
        #expect(failureModel.startInstall(testRequest()))
        await failureModel.waitForCurrentInstall()
        #expect(failureModel.installState.phase == .failed)
        #expect(failureModel.installState.errorMessage == "network unavailable")
        #expect(failureModel.history.first?.result == .failed)
    }

    @Test
    func recordsCancellation() async {
        let backend = MockInstallerBackend()
        backend.behavior = .wait
        let model = makeModel(backend: backend)
        #expect(model.startInstall(testRequest()))
        await Task.yield()

        model.cancelInstall()
        await model.waitForCurrentInstall()

        #expect(model.installState.phase == .cancelled)
        #expect(model.history.first?.result == .cancelled)
    }

    @Test
    func recoversPendingTaskAsInterrupted() {
        let persistence = MemoryPersistence()
        persistence.pending = PendingInstall(startedAt: .now.addingTimeInterval(-60), targetVersion: "1.2.3")
        let model = makeModel(backend: MockInstallerBackend(), persistence: persistence)

        #expect(model.installState.phase == .interrupted)
        #expect(model.history.first?.result == .interrupted)
        #expect(persistence.pending == nil)
    }

    @Test
    func menuAndWindowShareOneStateSource() {
        let model = makeModel(backend: MockInstallerBackend())
        let runtime = AppRuntime(model: model, launchAtLogin: LaunchAtLoginController(service: MockLoginItemService()))

        #expect(runtime.menuModel === runtime.windowModel)
    }

    @Test
    func startsAndStopsTheDetectedLaunchAgent() async {
        let detector = StubStatusDetector()
        detector.status = installedStatus(running: false)
        let service = MockMachineServiceController()
        service.onStart = { detector.status = installedStatus(running: true) }
        service.onStop = { detector.status = installedStatus(running: false) }
        let model = makeModel(
            backend: MockInstallerBackend(),
            detector: detector,
            machineService: service
        )

        #expect(model.startMachine())
        await model.waitForServiceAction()
        #expect(model.isMachineRunning)
        #expect(service.startCount == 1)

        #expect(model.stopMachine())
        await model.waitForServiceAction()
        #expect(!model.isMachineRunning)
        #expect(service.stopCount == 1)
    }

    @Test
    func signsInThenAppliesDependencyPlan() async {
        let persistence = MemoryPersistence()
        persistence.settings.controllerURL = "https://cowboy.example"
        let detector = StubStatusDetector()
        detector.status = installedStatus(running: true)
        let client = MockCowboyServiceClient()
        let update = DependencyUpdateItem(
            component: .init(kind: "provider_cli", slot: "codex"),
            displayName: "Codex CLI",
            currentVersion: "1.0.0",
            targetVersion: "1.1.0",
            activeLeases: 0,
            channel: .npm
        )
        client.plan = DependencyUpdatePlan(
            machineID: "macbook-air",
            machineName: "MacBook Air",
            activeSessions: 0,
            items: [update]
        )
        let model = makeModel(
            backend: MockInstallerBackend(),
            persistence: persistence,
            detector: detector,
            serviceClient: client
        )

        #expect(model.signIn(account: "owner", password: "password"))
        await model.waitForAccountAction()
        #expect(model.accountStatus.canManageDependencies)

        #expect(model.checkDependencies())
        await model.waitForDependencyAction()
        #expect(model.dependencyUpdateState.plan?.items == [update])

        #expect(model.applyDependencyUpdates())
        await model.waitForDependencyAction()
        #expect(client.applied == [update])
        #expect(model.dependencyUpdateState.phase == .succeeded)
    }
}

private func installedStatus(running: Bool) -> InstalledStatus {
    InstalledStatus(
        isInstalled: true,
        version: "1.2.3",
        location: "/state",
        serviceOrigin: "https://cowboy.example",
        machineID: "macbook-air",
        launchAgentLabel: "xyz.stormbird.cowboy-machine",
        launchAgentPath: "/Users/test/Library/LaunchAgents/xyz.stormbird.cowboy-machine.plist",
        launchAgentLoaded: running
    )
}
