@preconcurrency import Foundation
import Darwin

public struct LaunchctlResult: Equatable, Sendable {
    public let status: Int32
    public let output: String

    public init(status: Int32, output: String) {
        self.status = status
        self.output = output
    }
}

public protocol LaunchctlRunning: Sendable {
    func run(arguments: [String]) async throws -> LaunchctlResult
}

public final class SystemLaunchctlRunner: LaunchctlRunning, @unchecked Sendable {
    public init() {}

    public func run(arguments: [String]) async throws -> LaunchctlResult {
        try await Task.detached(priority: .userInitiated) {
            let process = Process()
            process.executableURL = URL(fileURLWithPath: "/bin/launchctl")
            process.arguments = arguments
            let output = Pipe()
            process.standardOutput = output
            process.standardError = output
            try process.run()
            process.waitUntilExit()
            let data = output.fileHandleForReading.readDataToEndOfFile()
            return LaunchctlResult(
                status: process.terminationStatus,
                output: String(data: data, encoding: .utf8)?
                    .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            )
        }.value
    }
}

public protocol MachineServiceControlling: Sendable {
    func start(_ installedStatus: InstalledStatus) async throws
    func stop(_ installedStatus: InstalledStatus) async throws
}

public enum MachineServiceControllerError: LocalizedError, Equatable {
    case notInstalled
    case launchAgentMissing
    case invalidLaunchAgent
    case commandFailed(String)

    public var errorDescription: String? {
        switch self {
        case .notInstalled:
            "Install Cowboy Machine before starting its background service."
        case .launchAgentMissing:
            "Cowboy Machine is installed, but its LaunchAgent is missing. Re-enroll it in background mode."
        case .invalidLaunchAgent:
            "Cowboy refused to manage a LaunchAgent outside its user-scoped installation."
        case let .commandFailed(message):
            message
        }
    }
}

public final class LaunchctlMachineServiceController: MachineServiceControlling, @unchecked Sendable {
    private static let labelPrefix = "xyz.stormbird.cowboy-machine"

    private let runner: LaunchctlRunning
    private let fileManager: FileManager
    private let uid: UInt32
    private let launchAgentDirectory: URL

    public init(
        runner: LaunchctlRunning = SystemLaunchctlRunner(),
        fileManager: FileManager = .default,
        uid: UInt32 = getuid(),
        launchAgentDirectory: URL? = nil
    ) {
        self.runner = runner
        self.fileManager = fileManager
        self.uid = uid
        self.launchAgentDirectory = launchAgentDirectory
            ?? fileManager.homeDirectoryForCurrentUser
                .appendingPathComponent("Library/LaunchAgents", isDirectory: true)
    }

    public func start(_ installedStatus: InstalledStatus) async throws {
        let service = try validatedService(from: installedStatus)
        let current = try await runner.run(arguments: ["print", service.target])
        if current.status == 0 {
            if Self.isRunning(current.output) {
                return
            }
            try await checked(
                ["kickstart", "-k", service.target],
                failure: "Could not start Cowboy Machine"
            )
            return
        }
        try await checked(
            ["bootstrap", service.domain, service.plist.path],
            failure: "Could not load Cowboy Machine"
        )
    }

    public func stop(_ installedStatus: InstalledStatus) async throws {
        let service = try validatedService(from: installedStatus)
        let result = try await runner.run(arguments: ["bootout", service.target])
        if result.status == 0 {
            return
        }
        let current = try await runner.run(arguments: ["print", service.target])
        if current.status != 0 {
            return
        }
        throw MachineServiceControllerError.commandFailed(
            Self.commandFailure("Could not stop Cowboy Machine", result: result)
        )
    }

    private func checked(_ arguments: [String], failure: String) async throws {
        let result = try await runner.run(arguments: arguments)
        guard result.status == 0 else {
            throw MachineServiceControllerError.commandFailed(Self.commandFailure(failure, result: result))
        }
    }

    private func validatedService(from status: InstalledStatus) throws -> LaunchAgentService {
        guard status.isInstalled else { throw MachineServiceControllerError.notInstalled }
        guard let label = status.launchAgentLabel,
              let plistPath = status.launchAgentPath
        else {
            throw MachineServiceControllerError.launchAgentMissing
        }
        let allowedLabel = label == Self.labelPrefix || label.hasPrefix("\(Self.labelPrefix).svc-")
        let launchAgentDirectory = launchAgentDirectory.standardizedFileURL.path
        let plist = URL(fileURLWithPath: plistPath).standardizedFileURL
        guard allowedLabel,
              plist.deletingLastPathComponent().path == launchAgentDirectory,
              plist.lastPathComponent == "\(label).plist",
              fileManager.fileExists(atPath: plist.path)
        else {
            throw MachineServiceControllerError.invalidLaunchAgent
        }
        let domain = "gui/\(uid)"
        return LaunchAgentService(
            domain: domain,
            target: "\(domain)/\(label)",
            plist: plist
        )
    }

    private static func isRunning(_ output: String) -> Bool {
        output.contains("state = running") || output.range(
            of: #"\bpid\s*=\s*\d+"#,
            options: .regularExpression
        ) != nil
    }

    private static func commandFailure(_ prefix: String, result: LaunchctlResult) -> String {
        if result.output.isEmpty {
            return "\(prefix) (launchctl exited with status \(result.status))."
        }
        return "\(prefix): \(String(result.output.suffix(2_000)))"
    }
}

private struct LaunchAgentService {
    let domain: String
    let target: String
    let plist: URL
}
