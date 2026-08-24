import Foundation

public enum InstallPhase: String, Codable, Equatable, Sendable {
    case idle
    case preparing
    case validating
    case installing
    case activating
    case refreshing
    case succeeded
    case failed
    case cancelled
    case interrupted

    public var isRunning: Bool {
        switch self {
        case .preparing, .validating, .installing, .activating, .refreshing:
            true
        default:
            false
        }
    }
}

public struct InstallState: Codable, Equatable, Sendable {
    public var phase: InstallPhase
    public var progress: Double
    public var message: String
    public var errorMessage: String?

    public init(
        phase: InstallPhase = .idle,
        progress: Double = 0,
        message: String = "Ready",
        errorMessage: String? = nil
    ) {
        self.phase = phase
        self.progress = progress
        self.message = message
        self.errorMessage = errorMessage
    }
}

public struct InstallProgress: Equatable, Sendable {
    public let phase: InstallPhase
    public let fraction: Double
    public let message: String

    public init(phase: InstallPhase, fraction: Double, message: String) {
        self.phase = phase
        self.fraction = fraction
        self.message = message
    }
}

public struct InstallRequest: Equatable, Sendable {
    public let controllerURL: String
    public let enrollmentToken: String
    public let workspaceDirectory: String
    public let stateDirectory: String?
    public let targetVersion: String

    public init(
        controllerURL: String,
        enrollmentToken: String,
        workspaceDirectory: String,
        stateDirectory: String?,
        targetVersion: String
    ) {
        self.controllerURL = controllerURL
        self.enrollmentToken = enrollmentToken
        self.workspaceDirectory = workspaceDirectory
        self.stateDirectory = stateDirectory
        self.targetVersion = targetVersion
    }
}

public struct InstallReceipt: Equatable, Sendable {
    public let installedVersion: String
    public let installationDirectory: String
    public let log: String

    public init(installedVersion: String, installationDirectory: String, log: String) {
        self.installedVersion = installedVersion
        self.installationDirectory = installationDirectory
        self.log = log
    }
}

public struct InstalledStatus: Codable, Equatable, Sendable {
    public let isInstalled: Bool
    public let version: String?
    public let location: String?
    public let serviceOrigin: String?
    public let launchAgentLoaded: Bool

    public init(
        isInstalled: Bool,
        version: String? = nil,
        location: String? = nil,
        serviceOrigin: String? = nil,
        launchAgentLoaded: Bool = false
    ) {
        self.isInstalled = isInstalled
        self.version = version
        self.location = location
        self.serviceOrigin = serviceOrigin
        self.launchAgentLoaded = launchAgentLoaded
    }

    public static let notInstalled = InstalledStatus(isInstalled: false)
}

public enum ActivityResult: String, Codable, Equatable, Sendable {
    case succeeded
    case failed
    case cancelled
    case interrupted
}

public struct ActivityRecord: Codable, Equatable, Identifiable, Sendable {
    public let id: UUID
    public let startedAt: Date
    public let endedAt: Date
    public let targetVersion: String
    public let result: ActivityResult
    public let summary: String
    public let details: String

    public init(
        id: UUID = UUID(),
        startedAt: Date,
        endedAt: Date,
        targetVersion: String,
        result: ActivityResult,
        summary: String,
        details: String
    ) {
        self.id = id
        self.startedAt = startedAt
        self.endedAt = endedAt
        self.targetVersion = targetVersion
        self.result = result
        self.summary = summary
        self.details = details
    }
}

public struct InstallerSettings: Codable, Equatable, Sendable {
    public var controllerURL: String
    public var workspaceDirectory: String
    public var stateDirectory: String
    public var automaticallyCheckForUpdates: Bool
    public var notificationsEnabled: Bool

    public init(
        controllerURL: String = "",
        workspaceDirectory: String = FileManager.default.homeDirectoryForCurrentUser.path,
        stateDirectory: String = "",
        automaticallyCheckForUpdates: Bool = true,
        notificationsEnabled: Bool = false
    ) {
        self.controllerURL = controllerURL
        self.workspaceDirectory = workspaceDirectory
        self.stateDirectory = stateDirectory
        self.automaticallyCheckForUpdates = automaticallyCheckForUpdates
        self.notificationsEnabled = notificationsEnabled
    }
}

public struct PendingInstall: Codable, Equatable, Sendable {
    public let startedAt: Date
    public let targetVersion: String

    public init(startedAt: Date, targetVersion: String) {
        self.startedAt = startedAt
        self.targetVersion = targetVersion
    }
}
