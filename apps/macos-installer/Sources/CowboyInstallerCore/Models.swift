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
    public let serviceID: String?
    public let machineID: String?
    public let launchAgentLabel: String?
    public let launchAgentPath: String?
    public let launchAgentLoaded: Bool

    public init(
        isInstalled: Bool,
        version: String? = nil,
        location: String? = nil,
        serviceOrigin: String? = nil,
        serviceID: String? = nil,
        machineID: String? = nil,
        launchAgentLabel: String? = nil,
        launchAgentPath: String? = nil,
        launchAgentLoaded: Bool = false
    ) {
        self.isInstalled = isInstalled
        self.version = version
        self.location = location
        self.serviceOrigin = serviceOrigin
        self.serviceID = serviceID
        self.machineID = machineID
        self.launchAgentLabel = launchAgentLabel
        self.launchAgentPath = launchAgentPath
        self.launchAgentLoaded = launchAgentLoaded
    }

    public static let notInstalled = InstalledStatus(isInstalled: false)
}

public enum MachineServiceActionPhase: String, Equatable, Sendable {
    case idle
    case starting
    case stopping
    case failed

    public var isRunning: Bool {
        self == .starting || self == .stopping
    }
}

public struct MachineServiceActionState: Equatable, Sendable {
    public var phase: MachineServiceActionPhase
    public var message: String
    public var errorMessage: String?

    public init(
        phase: MachineServiceActionPhase = .idle,
        message: String = "Ready",
        errorMessage: String? = nil
    ) {
        self.phase = phase
        self.message = message
        self.errorMessage = errorMessage
    }
}

public enum AccountPhase: String, Equatable, Sendable {
    case unknown
    case checking
    case localOwner
    case signedOut
    case signedIn
    case setupRequired
    case failed
}

public struct AccountSignInProvider: Equatable, Sendable, Identifiable {
    public let id: String
    public let displayName: String
    public let startPath: String

    public init(id: String, displayName: String, startPath: String) {
        self.id = id
        self.displayName = displayName
        self.startPath = startPath
    }
}

public struct PasskeySessionRefreshStatus: Equatable, Sendable {
    public var registeredCount: Int
    public var enabled: Bool
    public var intervalMilliseconds: Int64

    public init(registeredCount: Int, enabled: Bool, intervalMilliseconds: Int64) {
        self.registeredCount = registeredCount
        self.enabled = enabled
        self.intervalMilliseconds = intervalMilliseconds
    }
}

public struct OidcAuthorizationRequest: Equatable, Sendable {
    public let providerID: String
    public let launchURL: URL
    public let codeVerifier: String
    public let usesLegacyRoutes: Bool

    public init(
        providerID: String,
        launchURL: URL,
        codeVerifier: String,
        usesLegacyRoutes: Bool = false
    ) {
        self.providerID = providerID
        self.launchURL = launchURL
        self.codeVerifier = codeVerifier
        self.usesLegacyRoutes = usesLegacyRoutes
    }
}

public struct AccountStatus: Equatable, Sendable {
    public var phase: AccountPhase
    public var account: String?
    public var role: String?
    public var administratorAccess: Bool
    public var passwordEnabled: Bool
    public var signInProviders: [AccountSignInProvider]
    public var passkeySessionRefresh: PasskeySessionRefreshStatus?
    public var message: String
    public var errorMessage: String?

    public init(
        phase: AccountPhase = .unknown,
        account: String? = nil,
        role: String? = nil,
        administratorAccess: Bool = false,
        passwordEnabled: Bool = true,
        signInProviders: [AccountSignInProvider] = [],
        passkeySessionRefresh: PasskeySessionRefreshStatus? = nil,
        message: String = "Account status has not been checked",
        errorMessage: String? = nil
    ) {
        self.phase = phase
        self.account = account
        self.role = role
        self.administratorAccess = administratorAccess
        self.passwordEnabled = passwordEnabled
        self.signInProviders = signInProviders
        self.passkeySessionRefresh = passkeySessionRefresh
        self.message = message
        self.errorMessage = errorMessage
    }

    public var canReadProduct: Bool {
        phase == .localOwner || phase == .signedIn
    }

    public var authenticationIsOptional: Bool {
        phase == .localOwner
    }

    public var canManageDependencies: Bool {
        canReadProduct && administratorAccess
    }
}

public struct MachineComponentIdentifier: Codable, Equatable, Hashable, Sendable {
    public let kind: String
    public let slot: String?

    public init(kind: String, slot: String? = nil) {
        self.kind = kind
        self.slot = slot
    }
}

public struct MachineComponentReleaseUpdate: Codable, Equatable, Sendable {
    public let latestVersion: String
    public let available: Bool
    public let source: String
    public let checkedAtMilliseconds: Int64
    public let installable: Bool

    enum CodingKeys: String, CodingKey {
        case latestVersion = "latest_version"
        case available
        case source
        case checkedAtMilliseconds = "checked_at_ms"
        case installable
    }

    public init(
        latestVersion: String,
        available: Bool,
        source: String,
        checkedAtMilliseconds: Int64,
        installable: Bool
    ) {
        self.latestVersion = latestVersion
        self.available = available
        self.source = source
        self.checkedAtMilliseconds = checkedAtMilliseconds
        self.installable = installable
    }
}

public struct MachineComponentSummary: Codable, Equatable, Sendable {
    public let id: MachineComponentIdentifier
    public let state: String
    public let version: String
    public let generation: String
    public let activeLeases: UInt64
    public let update: MachineComponentReleaseUpdate?

    enum CodingKeys: String, CodingKey {
        case id
        case state
        case version
        case generation
        case activeLeases = "active_leases"
        case update
    }

    public init(
        id: MachineComponentIdentifier,
        state: String,
        version: String,
        generation: String,
        activeLeases: UInt64,
        update: MachineComponentReleaseUpdate? = nil
    ) {
        self.id = id
        self.state = state
        self.version = version
        self.generation = generation
        self.activeLeases = activeLeases
        self.update = update
    }
}

public enum ManagedMachineHealthState: String, Equatable, Sendable {
    case ready
    case starting
    case reconnecting
    case updating
    case degraded
    case offline
}

/// Server-authoritative Machine health. String wire values remain open so a
/// newer Controller cannot make an older Manager reject the whole registry.
public struct ManagedMachineHealth: Codable, Equatable, Sendable {
    public let state: String
    public let reason: String
    public let observedAtMilliseconds: Int64
    public let lastSeenAtMilliseconds: Int64?

    enum CodingKeys: String, CodingKey {
        case state
        case reason
        case observedAtMilliseconds = "observed_at_ms"
        case lastSeenAtMilliseconds = "last_seen_at_ms"
    }

    public init(
        state: String,
        reason: String,
        observedAtMilliseconds: Int64,
        lastSeenAtMilliseconds: Int64? = nil
    ) {
        self.state = state
        self.reason = reason
        self.observedAtMilliseconds = observedAtMilliseconds
        self.lastSeenAtMilliseconds = lastSeenAtMilliseconds
    }
}

public struct ManagedMachineSummary: Codable, Equatable, Sendable {
    public let id: String
    public let displayName: String
    public let status: String
    public let connected: Bool
    public let health: ManagedMachineHealth?
    public let activeSessions: UInt32
    public let pendingUpdates: [MachineComponentIdentifier]
    public let components: [MachineComponentSummary]

    enum CodingKeys: String, CodingKey {
        case id
        case displayName = "display_name"
        case status
        case connected
        case health
        case activeSessions = "active_sessions"
        case pendingUpdates = "pending_updates"
        case components
    }

    public init(
        id: String,
        displayName: String,
        status: String,
        connected: Bool,
        health: ManagedMachineHealth? = nil,
        activeSessions: UInt32,
        pendingUpdates: [MachineComponentIdentifier],
        components: [MachineComponentSummary]
    ) {
        self.id = id
        self.displayName = displayName
        self.status = status
        self.connected = connected
        self.health = health
        self.activeSessions = activeSessions
        self.pendingUpdates = pendingUpdates
        self.components = components
    }

    /// Prefer the Controller's projection. The fallback keeps Manager usable
    /// during a rolling Controller upgrade and still relies only on server
    /// fields, never local LaunchAgent process presence.
    public var effectiveHealthState: ManagedMachineHealthState {
        if let health {
            return ManagedMachineHealthState(rawValue: health.state) ?? .degraded
        }
        switch status {
        case "online": connected ? .ready : .degraded
        case "reconnecting": .reconnecting
        case "updating": .updating
        case "degraded": .degraded
        case "offline": .offline
        default: .degraded
        }
    }

    public var healthDisplayName: String {
        switch effectiveHealthState {
        case .ready: "Ready"
        case .starting: "Starting"
        case .reconnecting: "Reconnecting"
        case .updating: "Updating"
        case .degraded: "Not responding"
        case .offline: "Offline"
        }
    }
}

public enum DependencyUpdateChannel: String, Codable, Equatable, Sendable {
    case signedComponent
    case npm
}

public struct DependencyUpdateItem: Codable, Equatable, Identifiable, Sendable {
    public let component: MachineComponentIdentifier
    public let displayName: String
    public let currentVersion: String
    public let targetVersion: String
    public let activeLeases: UInt64
    public let channel: DependencyUpdateChannel

    public var id: String {
        "\(channel.rawValue):\(component.kind):\(component.slot ?? "")"
    }

    public init(
        component: MachineComponentIdentifier,
        displayName: String,
        currentVersion: String,
        targetVersion: String,
        activeLeases: UInt64,
        channel: DependencyUpdateChannel
    ) {
        self.component = component
        self.displayName = displayName
        self.currentVersion = currentVersion
        self.targetVersion = targetVersion
        self.activeLeases = activeLeases
        self.channel = channel
    }
}

public struct DependencyUpdatePlan: Equatable, Sendable {
    public let machineID: String
    public let machineName: String
    public let activeSessions: UInt32
    public let items: [DependencyUpdateItem]

    public init(
        machineID: String,
        machineName: String,
        activeSessions: UInt32,
        items: [DependencyUpdateItem]
    ) {
        self.machineID = machineID
        self.machineName = machineName
        self.activeSessions = activeSessions
        self.items = items
    }

    public var requiresConfirmation: Bool {
        activeSessions > 0 && items.contains(where: { $0.activeLeases > 0 })
    }
}

public enum DependencyUpdatePhase: String, Equatable, Sendable {
    case idle
    case checking
    case ready
    case updating
    case succeeded
    case failed

    public var isRunning: Bool {
        self == .checking || self == .updating
    }
}

public struct DependencyUpdateState: Equatable, Sendable {
    public var phase: DependencyUpdatePhase
    public var progress: Double
    public var message: String
    public var plan: DependencyUpdatePlan?
    public var errorMessage: String?

    public init(
        phase: DependencyUpdatePhase = .idle,
        progress: Double = 0,
        message: String = "Updates have not been checked",
        plan: DependencyUpdatePlan? = nil,
        errorMessage: String? = nil
    ) {
        self.phase = phase
        self.progress = progress
        self.message = message
        self.plan = plan
        self.errorMessage = errorMessage
    }
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
