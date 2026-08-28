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
        #expect(status.canManageDependencies)
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
