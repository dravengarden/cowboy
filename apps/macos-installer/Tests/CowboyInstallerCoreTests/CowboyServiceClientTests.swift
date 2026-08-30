import CowboyInstallerCore
import Foundation
import Testing

struct CowboyServiceClientTests {
    @Test
    func mapsAuthOffLocalOwnerAndAdminSession() async throws {
        let transport = SequenceHTTPTransport(responses: [
            jsonResponse(#"{"setup_required":false,"me":{"account":"local","role":"owner","auth_enabled":false}}"#),
            jsonResponse(#"{"authenticated":true,"role":"owner"}"#),
        ])
        let client = URLSessionCowboyServiceClient(transport: transport)

        let status = try await client.accountStatus(controllerURL: "https://cowboy.example")

        #expect(status.phase == .localOwner)
        #expect(status.account == "local")
        #expect(status.authenticationIsOptional)
        #expect(status.canManageDependencies)
    }

    @Test
    func authOffLocalOwnerDoesNotRequireImmediateAuthentication() async throws {
        let transport = SequenceHTTPTransport(responses: [
            jsonResponse(#"{"setup_required":false,"me":{"account":"local","role":"owner","auth_enabled":false}}"#),
            jsonResponse(#"{"authenticated":false,"role":null}"#),
        ])
        let client = URLSessionCowboyServiceClient(transport: transport)

        let status = try await client.accountStatus(controllerURL: "https://cowboy.example")

        #expect(status.phase == .localOwner)
        #expect(status.authenticationIsOptional)
        #expect(status.canReadProduct)
        #expect(!status.canManageDependencies)
        #expect(status.message == "Local access is enabled. Authentication is optional and can be configured later.")
    }

    @Test
    func passwordSignInCreatesProductAndAdminSessionsWithOrigin() async throws {
        let transport = SequenceHTTPTransport(responses: [
            jsonResponse(#"{"setup_required":false}"#),
            jsonResponse(#"{"account":"owner","role":"owner"}"#),
            jsonResponse(#"{"authenticated":true,"role":"owner"}"#),
            jsonResponse(#"{"setup_required":false,"me":{"account":"owner","role":"owner"}}"#),
            jsonResponse(#"{"authenticated":true,"role":"owner"}"#),
        ])
        let client = URLSessionCowboyServiceClient(transport: transport)

        let status = try await client.signIn(
            controllerURL: "https://cowboy.example",
            account: "owner",
            password: "correct horse battery staple"
        )
        let requests = await transport.requests

        #expect(status.phase == .signedIn)
        #expect(status.canManageDependencies)
        #expect(requests.map(\.url?.path) == [
            "/api/auth/status",
            "/api/auth/login",
            "/api/admin/auth/login",
            "/api/auth/status",
            "/api/admin/auth",
        ])
        #expect(requests[1].value(forHTTPHeaderField: "Origin") == "https://cowboy.example")
        #expect(requests[2].value(forHTTPHeaderField: "Origin") == "https://cowboy.example")
    }

    @Test
    func providerAuthorizationUsesProviderBoundNativePKCEHandoff() async throws {
        let transport = SequenceHTTPTransport(responses: [
            jsonResponse(#"{"setup_required":false,"password_enabled":false,"providers":[{"id":"cardea","display_name":"Cardea","start_url":"/api/auth/oidc/start"}]}"#),
            jsonResponse(#"{"authenticated":false,"role":null}"#),
        ])
        let client = URLSessionCowboyServiceClient(transport: transport)
        let status = try await client.accountStatus(controllerURL: "https://cowboy.example")
        let provider = try #require(status.signInProviders.first)
        #expect(!status.passwordEnabled)

        let authorization = try client.oidcAuthorizationRequest(
            controllerURL: "https://cowboy.example",
            provider: provider
        )
        let components = try #require(URLComponents(
            url: authorization.launchURL,
            resolvingAgainstBaseURL: false
        ))
        let query = Dictionary(uniqueKeysWithValues: (components.queryItems ?? []).compactMap {
            item in item.value.map { (item.name, $0) }
        })

        #expect(authorization.launchURL.scheme == "https")
        #expect(authorization.launchURL.host == "cowboy.example")
        #expect(authorization.providerID == "cardea")
        #expect(authorization.launchURL.path == "/api/auth/oidc/start")
        #expect(authorization.usesLegacyRoutes)
        #expect(query["client"] == "macos-manager")
        #expect(query["code_challenge"]?.count == 43)
        #expect(authorization.codeVerifier.count == 43)
        #expect(query["code_challenge"] != authorization.codeVerifier)
    }

    @Test
    func scopedProviderAuthorizationCannotEscapeItsProviderPath() throws {
        let client = URLSessionCowboyServiceClient(
            transport: SequenceHTTPTransport(responses: [])
        )
        let google = AccountSignInProvider(
            id: "google",
            displayName: "Google",
            startPath: "/api/auth/providers/google/start"
        )
        let authorization = try client.oidcAuthorizationRequest(
            controllerURL: "https://cowboy.example",
            provider: google
        )
        #expect(authorization.providerID == "google")
        #expect(authorization.launchURL.path == "/api/auth/providers/google/start")
        #expect(!authorization.usesLegacyRoutes)

        let forged = AccountSignInProvider(
            id: "google",
            displayName: "Google",
            startPath: "/api/auth/providers/cardea/start"
        )
        #expect(throws: CowboyServiceClientError.invalidResponse) {
            try client.oidcAuthorizationRequest(
                controllerURL: "https://cowboy.example",
                provider: forged
            )
        }
    }

    @Test
    func nativeHandoffExchangeSignsInProductAndAdministratorSessions() async throws {
        let transport = SequenceHTTPTransport(responses: [
            jsonResponse(#"{"account":"owner","role":"owner"}"#),
            jsonResponse(#"{"setup_required":false,"providers":[{"id":"cardea","display_name":"Cardea","start_url":"/api/auth/oidc/start"}],"me":{"account":"owner","role":"owner","passkey_count":0,"passkey_reauth_enabled":false,"passkey_reauth_after_ms":604800000}}"#),
            jsonResponse(#"{"authenticated":true,"role":"owner"}"#),
        ])
        let client = URLSessionCowboyServiceClient(transport: transport)
        let callback = URL(string: "xyz.stormbird.cowboy.manager://auth/callback?code=\(String(repeating: "a", count: 64))")!

        let status = try await client.completeOidcAuthorization(
            controllerURL: "https://cowboy.example",
            callbackURL: callback,
            providerID: "cardea",
            usesLegacyRoutes: true,
            codeVerifier: String(repeating: "b", count: 43)
        )
        let requests = await transport.requests
        let exchangeBody = try #require(requests[0].httpBody)
        let exchange = try #require(
            JSONSerialization.jsonObject(with: exchangeBody) as? [String: Any]
        )

        #expect(status.phase == .signedIn)
        #expect(status.canManageDependencies)
        #expect(status.passkeySessionRefresh?.enabled == false)
        #expect(requests.map(\.url?.path) == [
            "/api/auth/oidc/native/exchange",
            "/api/auth/status",
            "/api/admin/auth",
        ])
        #expect(requests[0].httpMethod == "POST")
        #expect(requests[0].value(forHTTPHeaderField: "Origin") == "https://cowboy.example")
        #expect(exchange["code"] as? String == String(repeating: "a", count: 64))
        #expect(exchange["code_verifier"] as? String == String(repeating: "b", count: 43))
    }

    @Test
    func nativeHandoffRejectsForeignCallbackSchemesWithoutNetwork() async {
        let transport = SequenceHTTPTransport(responses: [])
        let client = URLSessionCowboyServiceClient(transport: transport)

        await #expect(throws: CowboyServiceClientError.invalidAuthenticationCallback) {
            _ = try await client.completeOidcAuthorization(
                controllerURL: "https://cowboy.example",
                callbackURL: URL(string: "https://attacker.example/?code=abc")!,
                providerID: "cardea",
                usesLegacyRoutes: true,
                codeVerifier: String(repeating: "b", count: 43)
            )
        }
        #expect(await transport.requests.isEmpty)
    }

    @Test
    func passkeyRefreshSettingUsesBoundedServerPolicyEndpoint() async throws {
        let transport = SequenceHTTPTransport(responses: [
            jsonResponse(#"{"account":"owner","role":"owner","passkey_count":1,"passkey_reauth_enabled":true,"passkey_reauth_after_ms":86400000}"#),
            jsonResponse(#"{"setup_required":false,"me":{"account":"owner","role":"owner","passkey_count":1,"passkey_reauth_enabled":true,"passkey_reauth_after_ms":86400000}}"#),
            jsonResponse(#"{"authenticated":true,"role":"owner"}"#),
        ])
        let client = URLSessionCowboyServiceClient(transport: transport)

        let status = try await client.setPasskeySessionRefresh(
            controllerURL: "https://cowboy.example",
            enabled: true,
            intervalMilliseconds: 86_400_000
        )
        let requests = await transport.requests

        #expect(status.passkeySessionRefresh?.enabled == true)
        #expect(status.passkeySessionRefresh?.intervalMilliseconds == 86_400_000)
        #expect(requests[0].httpMethod == "PUT")
        #expect(requests[0].url?.path == "/api/auth/passkeys/reauth")
    }

    @Test
    func buildsOnePlanForSignedAndNpmUpdates() async throws {
        let transport = SequenceHTTPTransport(responses: [jsonResponse(#"""
        [
          {
            "id":"macbook-air",
            "display_name":"MacBook Air",
            "status":"online",
            "connected":true,
            "active_sessions":2,
            "pending_updates":[{"kind":"machine_host"}],
            "components":[
              {
                "id":{"kind":"machine_host"},
                "state":"active",
                "version":"0.1.0",
                "generation":"old",
                "active_leases":1
              },
              {
                "id":{"kind":"provider_cli","slot":"codex"},
                "state":"active",
                "version":"1.0.0",
                "generation":"",
                "active_leases":2,
                "update":{
                  "latest_version":"1.1.0",
                  "available":true,
                  "source":"npm registry",
                  "checked_at_ms":123,
                  "installable":true
                }
              }
            ]
          }
        ]
        """#)])
        let client = URLSessionCowboyServiceClient(transport: transport)

        let plan = try await client.dependencyUpdatePlan(
            controllerURL: "https://cowboy.example",
            machineID: "macbook-air",
            refresh: false
        )

        #expect(plan.items.count == 2)
        #expect(plan.items.map(\.channel) == [
            DependencyUpdateChannel.npm,
            DependencyUpdateChannel.signedComponent,
        ])
        #expect(plan.requiresConfirmation)
    }

    @Test
    func refreshedPlanUsesTheControllerReceiptWithoutPollingMachineEvents() async throws {
        let transport = SequenceHTTPTransport(responses: [
            jsonResponse(#"{"request_id":"refresh-1"}"#),
            jsonResponse(#"[{"id":"macbook-air","display_name":"MacBook Air","status":"online","connected":true,"active_sessions":0,"pending_updates":[],"components":[]}]"#),
        ])
        let client = URLSessionCowboyServiceClient(transport: transport)

        let plan = try await client.dependencyUpdatePlan(
            controllerURL: "https://cowboy.example",
            machineID: "macbook-air",
            refresh: true
        )
        let requests = await transport.requests

        #expect(plan.items.isEmpty)
        #expect(requests.map(\.url?.path) == [
            "/api/machines/macbook-air/refresh",
            "/api/machines",
        ])
        #expect(requests.map(\.httpMethod) == ["POST", "GET"])
        #expect(requests[0].timeoutInterval == 95)
        #expect(!requests.contains { $0.url?.path.hasSuffix("/events") == true })
    }

    @Test
    func dependencyUpdateUsesTheControllerReceiptWithoutPollingMachineEvents() async throws {
        let transport = SequenceHTTPTransport(responses: [
            jsonResponse(#"{"request_id":"update-npm-1"}"#),
        ])
        let client = URLSessionCowboyServiceClient(transport: transport)
        let item = DependencyUpdateItem(
            component: MachineComponentIdentifier(kind: "provider_cli", slot: "codex"),
            displayName: "Codex CLI",
            currentVersion: "1.0.0",
            targetVersion: "1.1.0",
            activeLeases: 0,
            channel: .npm
        )

        try await client.applyDependencyUpdate(
            controllerURL: "https://cowboy.example",
            machineID: "macbook-air",
            item: item
        )
        let requests = await transport.requests
        let body = try #require(requests.first?.httpBody)
        let component = try #require(
            JSONSerialization.jsonObject(with: body) as? [String: String]
        )

        #expect(requests.count == 1)
        #expect(requests[0].httpMethod == "POST")
        #expect(requests[0].url?.path == "/api/machines/macbook-air/components/update-npm")
        #expect(requests[0].timeoutInterval == 305)
        #expect(component == ["kind": "provider_cli", "slot": "codex"])
    }

    @Test
    func rejectsRemotePlaintextControllerURLs() async {
        let client = URLSessionCowboyServiceClient(transport: SequenceHTTPTransport(responses: []))

        await #expect(throws: CowboyServiceClientError.invalidControllerURL) {
            _ = try await client.accountStatus(controllerURL: "http://cowboy.example")
        }
    }
}

private actor SequenceHTTPTransport: ServiceHTTPTransport {
    private var responses: [ServiceHTTPResponse]
    private(set) var requests: [URLRequest] = []

    init(responses: [ServiceHTTPResponse]) {
        self.responses = responses
    }

    func send(_ request: URLRequest) async throws -> ServiceHTTPResponse {
        requests.append(request)
        guard !responses.isEmpty else {
            return ServiceHTTPResponse(statusCode: 500, data: Data("unexpected request".utf8))
        }
        return responses.removeFirst()
    }
}

private func jsonResponse(_ value: String, status: Int = 200) -> ServiceHTTPResponse {
    ServiceHTTPResponse(statusCode: status, data: Data(value.utf8))
}
