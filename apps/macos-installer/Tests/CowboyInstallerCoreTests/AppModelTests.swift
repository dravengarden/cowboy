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
}
