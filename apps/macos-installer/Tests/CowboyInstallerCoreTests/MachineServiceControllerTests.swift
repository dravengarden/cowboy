import CowboyInstallerCore
import Foundation
import Testing

struct MachineServiceControllerTests {
    @Test
    func bootstrapsAnUnloadedCowboyLaunchAgent() async throws {
        let fixture = try LaunchAgentFixture()
        defer { fixture.remove() }
        let runner = StubLaunchctlRunner(results: [
            .init(status: 113, output: "Could not find service"),
            .init(status: 0, output: ""),
        ])
        let controller = LaunchctlMachineServiceController(
            runner: runner,
            uid: 501,
            launchAgentDirectory: fixture.directory
        )

        try await controller.start(fixture.status(running: false))

        #expect(await runner.commands == [
            ["print", "gui/501/xyz.stormbird.cowboy-machine"],
            ["bootstrap", "gui/501", fixture.plist.path],
        ])
    }

    @Test
    func leavesAnAlreadyRunningMachineAlone() async throws {
        let fixture = try LaunchAgentFixture()
        defer { fixture.remove() }
        let runner = StubLaunchctlRunner(results: [
            .init(status: 0, output: "state = running\npid = 42"),
        ])
        let controller = LaunchctlMachineServiceController(
            runner: runner,
            uid: 501,
            launchAgentDirectory: fixture.directory
        )

        try await controller.start(fixture.status(running: true))

        #expect(await runner.commands.count == 1)
    }

    @Test
    func unloadsTheExactCowboyServiceTarget() async throws {
        let fixture = try LaunchAgentFixture()
        defer { fixture.remove() }
        let runner = StubLaunchctlRunner(results: [.init(status: 0, output: "")])
        let controller = LaunchctlMachineServiceController(
            runner: runner,
            uid: 501,
            launchAgentDirectory: fixture.directory
        )

        try await controller.stop(fixture.status(running: true))

        #expect(await runner.commands == [
            ["bootout", "gui/501/xyz.stormbird.cowboy-machine"],
        ])
    }

    @Test
    func rejectsAPlistOutsideTheUserLaunchAgentDirectory() async {
        let runner = StubLaunchctlRunner(results: [])
        let allowed = FileManager.default.temporaryDirectory
            .appendingPathComponent("allowed-\(UUID().uuidString)", isDirectory: true)
        let controller = LaunchctlMachineServiceController(
            runner: runner,
            uid: 501,
            launchAgentDirectory: allowed
        )
        let status = InstalledStatus(
            isInstalled: true,
            launchAgentLabel: "xyz.stormbird.cowboy-machine",
            launchAgentPath: "/tmp/not-cowboy.plist",
            launchAgentLoaded: true
        )

        await #expect(throws: MachineServiceControllerError.invalidLaunchAgent) {
            try await controller.stop(status)
        }
    }
}

private actor StubLaunchctlRunner: LaunchctlRunning {
    private var results: [LaunchctlResult]
    private(set) var commands: [[String]] = []

    init(results: [LaunchctlResult]) {
        self.results = results
    }

    func run(arguments: [String]) async throws -> LaunchctlResult {
        commands.append(arguments)
        guard !results.isEmpty else {
            return LaunchctlResult(status: 1, output: "unexpected command")
        }
        return results.removeFirst()
    }
}

private struct LaunchAgentFixture {
    let directory: URL
    let plist: URL

    init() throws {
        directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("CowboyLaunchAgentTests.\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        plist = directory.appendingPathComponent("xyz.stormbird.cowboy-machine.plist")
        try Data().write(to: plist)
    }

    func status(running: Bool) -> InstalledStatus {
        InstalledStatus(
            isInstalled: true,
            launchAgentLabel: "xyz.stormbird.cowboy-machine",
            launchAgentPath: plist.path,
            launchAgentLoaded: running
        )
    }

    func remove() {
        try? FileManager.default.removeItem(at: directory)
    }
}
