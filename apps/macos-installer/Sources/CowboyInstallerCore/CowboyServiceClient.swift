@preconcurrency import Foundation

public struct ServiceHTTPResponse: Equatable, Sendable {
    public let statusCode: Int
    public let data: Data

    public init(statusCode: Int, data: Data) {
        self.statusCode = statusCode
        self.data = data
    }
}

public protocol ServiceHTTPTransport: Sendable {
    func send(_ request: URLRequest) async throws -> ServiceHTTPResponse
}

public final class URLSessionHTTPTransport: ServiceHTTPTransport, @unchecked Sendable {
    private let providedSession: URLSession?
    private let sessionLock = NSLock()
    private var storedSession: URLSession?

    public init(session: URLSession? = nil) {
        providedSession = session
    }

    public func send(_ request: URLRequest) async throws -> ServiceHTTPResponse {
        let (data, response) = try await session().data(for: request)
        guard let response = response as? HTTPURLResponse else {
            throw CowboyServiceClientError.invalidResponse
        }
        return ServiceHTTPResponse(statusCode: response.statusCode, data: data)
    }

    private func session() -> URLSession {
        sessionLock.lock()
        defer { sessionLock.unlock() }
        if let providedSession {
            return providedSession
        }
        if let storedSession {
            return storedSession
        }
        let configuration = URLSessionConfiguration.default
        configuration.httpShouldSetCookies = true
        configuration.httpCookieAcceptPolicy = .always
        let session = URLSession(configuration: configuration)
        storedSession = session
        return session
    }
}

public protocol CowboyServiceClient: Sendable {
    func accountStatus(controllerURL: String) async throws -> AccountStatus
    func signIn(controllerURL: String, account: String, password: String) async throws -> AccountStatus
    func signOut(controllerURL: String) async throws -> AccountStatus
    func machine(controllerURL: String, machineID: String) async throws -> ManagedMachineSummary
    func dependencyUpdatePlan(
        controllerURL: String,
        machineID: String,
        refresh: Bool
    ) async throws -> DependencyUpdatePlan
    func applyDependencyUpdate(
        controllerURL: String,
        machineID: String,
        item: DependencyUpdateItem
    ) async throws
}

public enum CowboyServiceClientError: LocalizedError, Equatable {
    case invalidControllerURL
    case invalidResponse
    case credentialsRequired
    case setupRequired
    case machineNotFound(String)
    case requestFailed(Int, String)
    case commandFailed(String)
    case commandTimedOut

    public var errorDescription: String? {
        switch self {
        case .invalidControllerURL:
            "Enter a valid HTTPS Cowboy Service URL. Loopback HTTP is allowed for development."
        case .invalidResponse:
            "Cowboy Service returned an invalid response."
        case .credentialsRequired:
            "Enter the Cowboy owner account and password."
        case .setupRequired:
            "Complete first-time Cowboy setup in the browser before signing in here."
        case let .machineNotFound(machineID):
            "Cowboy Service does not currently list Machine \(machineID)."
        case let .requestFailed(status, message):
            message.isEmpty ? "Cowboy Service request failed with status \(status)." : message
        case let .commandFailed(message):
            message
        case .commandTimedOut:
            "Cowboy Machine did not report the command result before the timeout."
        }
    }
}

public final class URLSessionCowboyServiceClient: CowboyServiceClient, @unchecked Sendable {
    private let transport: ServiceHTTPTransport
    private let decoder = JSONDecoder()
    private let encoder = JSONEncoder()

    public init(transport: ServiceHTTPTransport = URLSessionHTTPTransport()) {
        self.transport = transport
    }

    public func accountStatus(controllerURL: String) async throws -> AccountStatus {
        let product: ProductAuthStatusDTO = try await get(controllerURL, path: "/api/auth/status")
        let admin: AdminAuthStatusDTO = try await get(controllerURL, path: "/api/admin/auth")
        return Self.accountStatus(product: product, admin: admin)
    }

    public func signIn(
        controllerURL: String,
        account: String,
        password: String
    ) async throws -> AccountStatus {
        let account = account.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !account.isEmpty, !password.isEmpty else {
            throw CowboyServiceClientError.credentialsRequired
        }
        let product: ProductAuthStatusDTO = try await get(controllerURL, path: "/api/auth/status")
        guard !product.setupRequired else { throw CowboyServiceClientError.setupRequired }
        let credentials = CredentialsDTO(account: account, password: password)
        if product.me?.authEnabled != false {
            let _: ProductMeDTO = try await post(
                controllerURL,
                path: "/api/auth/login",
                body: credentials
            )
        }
        let _: AdminAuthStatusDTO = try await post(
            controllerURL,
            path: "/api/admin/auth/login",
            body: credentials
        )
        return try await accountStatus(controllerURL: controllerURL)
    }

    public func signOut(controllerURL: String) async throws -> AccountStatus {
        let empty = EmptyBody()
        let _: IgnoredResponse = try await post(
            controllerURL,
            path: "/api/auth/logout",
            body: empty
        )
        let _: AdminAuthStatusDTO = try await post(
            controllerURL,
            path: "/api/admin/auth/logout",
            body: empty
        )
        return try await accountStatus(controllerURL: controllerURL)
    }

    public func machine(controllerURL: String, machineID: String) async throws -> ManagedMachineSummary {
        let machines: [ManagedMachineSummary] = try await get(controllerURL, path: "/api/machines")
        guard let machine = machines.first(where: { $0.id == machineID }) else {
            throw CowboyServiceClientError.machineNotFound(machineID)
        }
        return machine
    }

    public func dependencyUpdatePlan(
        controllerURL: String,
        machineID: String,
        refresh: Bool
    ) async throws -> DependencyUpdatePlan {
        if refresh {
            let command: MachineCommandResponseDTO = try await post(
                controllerURL,
                path: "/api/machines/\(escaped(machineID))/refresh",
                body: EmptyBody()
            )
            try await waitForCommand(
                controllerURL: controllerURL,
                machineID: machineID,
                requestID: command.requestID,
                attempts: 40,
                delay: .milliseconds(250)
            )
        }
        let machine = try await machine(controllerURL: controllerURL, machineID: machineID)
        let pending = Set(machine.pendingUpdates)
        let items = machine.components.compactMap { component -> DependencyUpdateItem? in
            if pending.contains(component.id) {
                return DependencyUpdateItem(
                    component: component.id,
                    displayName: Self.componentName(component.id),
                    currentVersion: component.version,
                    targetVersion: component.update?.latestVersion ?? "Signed release",
                    activeLeases: component.activeLeases,
                    channel: .signedComponent
                )
            }
            guard let update = component.update, update.available, update.installable else {
                return nil
            }
            return DependencyUpdateItem(
                component: component.id,
                displayName: Self.componentName(component.id),
                currentVersion: component.version,
                targetVersion: update.latestVersion,
                activeLeases: component.activeLeases,
                channel: .npm
            )
        }.sorted { $0.displayName.localizedStandardCompare($1.displayName) == .orderedAscending }
        return DependencyUpdatePlan(
            machineID: machine.id,
            machineName: machine.displayName,
            activeSessions: machine.activeSessions,
            items: items
        )
    }

    public func applyDependencyUpdate(
        controllerURL: String,
        machineID: String,
        item: DependencyUpdateItem
    ) async throws {
        let suffix = switch item.channel {
        case .signedComponent: "reconcile-one"
        case .npm: "update-npm"
        }
        let command: MachineCommandResponseDTO = try await post(
            controllerURL,
            path: "/api/machines/\(escaped(machineID))/components/\(suffix)",
            body: item.component
        )
        try await waitForCommand(
            controllerURL: controllerURL,
            machineID: machineID,
            requestID: command.requestID,
            attempts: 180,
            delay: .seconds(1)
        )
    }

    private func waitForCommand(
        controllerURL: String,
        machineID: String,
        requestID: String,
        attempts: Int,
        delay: Duration
    ) async throws {
        for _ in 0..<attempts {
            try Task.checkCancellation()
            try await Task.sleep(for: delay)
            let events: [MachineEventDTO] = try await get(
                controllerURL,
                path: "/api/machines/\(escaped(machineID))/events"
            )
            guard let result = events.first(where: {
                $0.event == "command_result" && $0.requestID == requestID
            }) else {
                continue
            }
            guard result.accepted == true else {
                throw CowboyServiceClientError.commandFailed(
                    result.detail ?? "Cowboy Machine rejected the command."
                )
            }
            return
        }
        throw CowboyServiceClientError.commandTimedOut
    }

    private func get<Response: Decodable>(_ controllerURL: String, path: String) async throws -> Response {
        let endpoint = try endpoint(controllerURL, path: path)
        var request = URLRequest(url: endpoint.url)
        request.httpMethod = "GET"
        request.cachePolicy = .reloadIgnoringLocalCacheData
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        return try await send(request)
    }

    private func post<Body: Encodable, Response: Decodable>(
        _ controllerURL: String,
        path: String,
        body: Body
    ) async throws -> Response {
        let endpoint = try endpoint(controllerURL, path: path)
        var request = URLRequest(url: endpoint.url)
        request.httpMethod = "POST"
        request.httpBody = try encoder.encode(body)
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue(endpoint.origin, forHTTPHeaderField: "Origin")
        return try await send(request)
    }

    private func send<Response: Decodable>(_ request: URLRequest) async throws -> Response {
        let response = try await transport.send(request)
        guard (200..<300).contains(response.statusCode) else {
            let message = String(data: response.data, encoding: .utf8)?
                .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            throw CowboyServiceClientError.requestFailed(
                response.statusCode,
                String(message.suffix(2_000))
            )
        }
        if Response.self == IgnoredResponse.self {
            return IgnoredResponse() as! Response
        }
        do {
            return try decoder.decode(Response.self, from: response.data)
        } catch {
            throw CowboyServiceClientError.invalidResponse
        }
    }

    private func endpoint(_ controllerURL: String, path: String) throws -> ServiceEndpoint {
        let trimmed = controllerURL.trimmingCharacters(in: .whitespacesAndNewlines)
        guard var components = URLComponents(string: trimmed),
              let scheme = components.scheme?.lowercased(),
              let host = components.host,
              components.user == nil,
              components.password == nil,
              components.query == nil,
              components.fragment == nil,
              scheme == "https" || (scheme == "http" && ["localhost", "127.0.0.1", "::1"].contains(host))
        else {
            throw CowboyServiceClientError.invalidControllerURL
        }
        components.path = path
        guard let url = components.url else { throw CowboyServiceClientError.invalidControllerURL }
        var origin = "\(scheme)://\(host.contains(":") ? "[\(host)]" : host)"
        if let port = components.port {
            origin += ":\(port)"
        }
        return ServiceEndpoint(url: url, origin: origin)
    }

    private func escaped(_ value: String) -> String {
        value.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) ?? value
    }

    private static func accountStatus(
        product: ProductAuthStatusDTO,
        admin: AdminAuthStatusDTO
    ) -> AccountStatus {
        let administratorAccess = admin.authenticated
            && ["owner", "operator"].contains(admin.role ?? "")
        if product.setupRequired {
            return AccountStatus(
                phase: .setupRequired,
                administratorAccess: administratorAccess,
                message: "Complete first-time setup in Cowboy before signing in here."
            )
        }
        if product.me?.authEnabled == false {
            return AccountStatus(
                phase: .localOwner,
                account: product.me?.account ?? "local",
                role: product.me?.role ?? "owner",
                administratorAccess: administratorAccess,
                message: administratorAccess
                    ? "Local access is enabled and administrator controls are unlocked."
                    : "Local access is enabled. Sign in to unlock dependency updates."
            )
        }
        if let me = product.me {
            return AccountStatus(
                phase: .signedIn,
                account: me.account,
                role: me.role,
                administratorAccess: administratorAccess,
                message: administratorAccess
                    ? "Signed in with administrator controls."
                    : "Signed in. Administrator access is required for dependency updates."
            )
        }
        return AccountStatus(
            phase: .signedOut,
            administratorAccess: administratorAccess,
            message: "Sign in to this Cowboy Service."
        )
    }

    private static func componentName(_ id: MachineComponentIdentifier) -> String {
        switch (id.kind, id.slot) {
        case ("machine_host", _): "Cowboy Machine"
        case ("acp_runtime", _): "ACP runtime"
        case ("zed_server", _): "Zed"
        case ("zed_adapter", _): "Zed adapter"
        case ("code_adapter", _): "Code adapter"
        case ("managed_node", _): "Managed Node"
        case ("provider_cli", let slot?): "\(slot.capitalized) CLI"
        case ("provider_adapter", let slot?): "\(slot.capitalized) adapter"
        default: id.slot ?? id.kind.replacingOccurrences(of: "_", with: " ").capitalized
        }
    }
}

private struct ServiceEndpoint {
    let url: URL
    let origin: String
}

private struct CredentialsDTO: Encodable {
    let account: String
    let password: String
}

private struct EmptyBody: Encodable {}

private struct IgnoredResponse: Decodable {}

private struct ProductAuthStatusDTO: Decodable {
    let setupRequired: Bool
    let me: ProductMeDTO?

    enum CodingKeys: String, CodingKey {
        case setupRequired = "setup_required"
        case me
    }
}

private struct ProductMeDTO: Decodable {
    let account: String
    let role: String
    let authEnabled: Bool?

    enum CodingKeys: String, CodingKey {
        case account
        case role
        case authEnabled = "auth_enabled"
    }
}

private struct AdminAuthStatusDTO: Decodable {
    let authenticated: Bool
    let role: String?
}

private struct MachineCommandResponseDTO: Decodable {
    let requestID: String

    enum CodingKeys: String, CodingKey {
        case requestID = "request_id"
    }
}

private struct MachineEventDTO: Decodable {
    let event: String
    let requestID: String?
    let accepted: Bool?
    let detail: String?

    enum CodingKeys: String, CodingKey {
        case event
        case requestID = "request_id"
        case accepted
        case detail
    }
}
