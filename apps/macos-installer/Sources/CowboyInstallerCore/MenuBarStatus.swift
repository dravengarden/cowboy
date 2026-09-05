public enum MenuBarStatusTone: Equatable, Sendable {
    case healthy
    case working
    case attention
    case inactive
}

public struct MenuBarStatus: Equatable, Sendable {
    public let symbolName: String
    public let message: String
    public let tone: MenuBarStatusTone

    public init(symbolName: String, message: String, tone: MenuBarStatusTone) {
        self.symbolName = symbolName
        self.message = message
        self.tone = tone
    }
}

extension AppModel {
    public var menuBarStatus: MenuBarStatus {
        if installState.phase.isRunning {
            return MenuBarStatus(
                symbolName: installState.menuBarSymbol,
                message: installState.message,
                tone: .working
            )
        }
        if serviceActionState.phase.isRunning {
            return MenuBarStatus(
                symbolName: "arrow.triangle.2.circlepath.circle.fill",
                message: serviceActionState.message,
                tone: .working
            )
        }
        if dependencyUpdateState.phase.isRunning {
            return MenuBarStatus(
                symbolName: "arrow.triangle.2.circlepath.circle.fill",
                message: dependencyUpdateState.message,
                tone: .working
            )
        }
        if serviceActionState.phase == .failed {
            return MenuBarStatus(
                symbolName: "exclamationmark.triangle.fill",
                message: serviceActionState.message,
                tone: .attention
            )
        }
        if let remoteMachine {
            switch remoteMachine.effectiveHealthState {
            case .ready:
                return MenuBarStatus(
                    symbolName: "bolt.horizontal.circle.fill",
                    message: "Cowboy Machine is running",
                    tone: .healthy
                )
            case .starting:
                return MenuBarStatus(
                    symbolName: "arrow.triangle.2.circlepath.circle.fill",
                    message: "Cowboy Machine is starting",
                    tone: .working
                )
            case .reconnecting:
                return MenuBarStatus(
                    symbolName: "arrow.triangle.2.circlepath.circle.fill",
                    message: "Cowboy Machine is reconnecting",
                    tone: .working
                )
            case .updating:
                return MenuBarStatus(
                    symbolName: "arrow.down.circle.fill",
                    message: "Cowboy Machine is updating",
                    tone: .working
                )
            case .degraded:
                return MenuBarStatus(
                    symbolName: "exclamationmark.triangle.fill",
                    message: "Cowboy Machine is not responding",
                    tone: .attention
                )
            case .offline:
                return MenuBarStatus(
                    symbolName: "bolt.slash.circle",
                    message: "Cowboy Machine is offline",
                    tone: .inactive
                )
            }
        }
        if isMachineRunning {
            return MenuBarStatus(
                symbolName: "questionmark.circle",
                message: "Cowboy Machine status is unavailable",
                tone: .attention
            )
        }
        if installedStatus.isInstalled {
            return MenuBarStatus(
                symbolName: "bolt.slash.circle",
                message: "Cowboy Machine is stopped",
                tone: .inactive
            )
        }
        return MenuBarStatus(
            symbolName: "arrow.down.circle",
            message: "Cowboy Machine is not installed",
            tone: .attention
        )
    }

    public var menuBarSymbol: String { menuBarStatus.symbolName }

    public var menuBarMessage: String { menuBarStatus.message }
}

extension InstallState {
    public var menuBarSymbol: String {
        switch phase {
        case .preparing, .validating, .installing, .activating, .refreshing:
            "arrow.down.circle"
        case .succeeded:
            "checkmark.circle.fill"
        case .failed, .interrupted:
            "exclamationmark.triangle.fill"
        case .cancelled:
            "xmark.circle.fill"
        case .idle:
            "shippingbox"
        }
    }
}
