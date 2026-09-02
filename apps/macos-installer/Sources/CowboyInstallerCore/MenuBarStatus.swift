public struct MenuBarStatus: Equatable, Sendable {
    public let symbolName: String
    public let message: String

    public init(symbolName: String, message: String) {
        self.symbolName = symbolName
        self.message = message
    }
}

extension AppModel {
    public var menuBarStatus: MenuBarStatus {
        if installState.phase.isRunning {
            return MenuBarStatus(
                symbolName: installState.menuBarSymbol,
                message: installState.message
            )
        }
        if serviceActionState.phase.isRunning {
            return MenuBarStatus(
                symbolName: "arrow.triangle.2.circlepath.circle.fill",
                message: serviceActionState.message
            )
        }
        if dependencyUpdateState.phase.isRunning {
            return MenuBarStatus(
                symbolName: "arrow.triangle.2.circlepath.circle.fill",
                message: dependencyUpdateState.message
            )
        }
        if serviceActionState.phase == .failed {
            return MenuBarStatus(
                symbolName: "exclamationmark.triangle.fill",
                message: serviceActionState.message
            )
        }
        if isMachineRunning {
            return MenuBarStatus(
                symbolName: "bolt.horizontal.circle.fill",
                message: "Cowboy Machine is running"
            )
        }
        if installedStatus.isInstalled {
            return MenuBarStatus(
                symbolName: "bolt.slash.circle",
                message: "Cowboy Machine is stopped"
            )
        }
        return MenuBarStatus(
            symbolName: "arrow.down.circle",
            message: "Cowboy Machine is not installed"
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
