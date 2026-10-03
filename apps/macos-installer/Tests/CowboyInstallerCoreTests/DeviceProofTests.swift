@testable import CowboyInstallerCore
import CryptoKit
import Foundation
import Security
import Testing

private final class ChallengeProtocol: URLProtocol, @unchecked Sendable {
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        guard let url = request.url, url.path == "/api/auth/browser/challenge" else {
            client?.urlProtocol(self, didFailWithError: URLError(.badURL))
            return
        }
        let response = HTTPURLResponse(url: url, statusCode: 200, httpVersion: nil, headerFields: nil)!
        let body = try! JSONSerialization.data(withJSONObject: [
            "epoch": UUID().uuidString,
            "server_time_ms": Int64(Date().timeIntervalSince1970 * 1000),
        ])
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: body)
        client?.urlProtocolDidFinishLoading(self)
    }
    override func stopLoading() {}
}

struct DeviceProofTests {
    private func session() -> URLSession {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [ChallengeProtocol.self]
        return URLSession(configuration: configuration)
    }

    private func decode(_ value: String) throws -> Data {
        let normalized = value.replacingOccurrences(of: "-", with: "+").replacingOccurrences(of: "_", with: "/")
        return try #require(Data(base64Encoded: normalized + String(repeating: "=", count: (4 - normalized.count % 4) % 4)))
    }

    private func proof(_ request: URLRequest) throws -> [String: Any] {
        let header = try #require(request.value(forHTTPHeaderField: "x-cowboy-browser-proof"))
        return try #require(JSONSerialization.jsonObject(with: decode(header)) as? [String: Any])
    }

    @Test
    func signaturesBindTheWireTargetAndSurviveReopeningTheDevice() async throws {
        let origin = "https://proof-\(UUID().uuidString.lowercased()).invalid"
        defer {
            // Delete only this test's unique Keychain item, including on failure.
            SecItemDelete([
                kSecClass as String: kSecClassGenericPassword,
                kSecAttrService as String: "xyz.stormbird.cowboy.manager.device",
                kSecAttrAccount as String: origin,
            ] as CFDictionary)
        }
        let session = session()
        defer { session.invalidateAndCancel() }
        var request = URLRequest(url: URL(string: origin + ":443/api/path%20with%20spaces?q=a%2Fb&x=%E4%BD%A0")!)
        request.httpMethod = "POST"
        let signer = DeviceProof()
        let first = try proof(await signer.sign(request, session: session))
        let second = try proof(await signer.sign(request, session: session))
        let refreshed = try proof(await signer.sign(request, session: session, refresh: true))
        let reopened = try proof(await DeviceProof().sign(request, session: session))
        #expect(first["origin"] as? String == origin)
        #expect(first["key"] as? String == reopened["key"] as? String)
        #expect(first["epoch"] as? String == second["epoch"] as? String)
        #expect(first["epoch"] as? String != refreshed["epoch"] as? String)
        #expect(first["nonce"] as? String != second["nonce"] as? String)
        let publicKey = try P256.Signing.PublicKey(x963Representation: decode(try #require(first["key"] as? String)))
        let signature = try P256.Signing.ECDSASignature(rawRepresentation: decode(try #require(first["signature"] as? String)))
        let fields = try ["epoch", "origin", "key"].map { try #require(first[$0] as? String) }
        let timestamp = try #require(first["time"] as? Int64)
        let nonce = try #require(first["nonce"] as? String)
        #expect(try decode(nonce).count == 32)
        let canonical = (["cowboy-browser-proof-v1"] + fields + [
            "POST", "/api/path%20with%20spaces?q=a%2Fb&x=%E4%BD%A0", String(timestamp), nonce,
        ]).joined(separator: "\n")
        #expect(publicKey.isValidSignature(signature, for: Data(canonical.utf8)))
        #expect(!publicKey.isValidSignature(signature, for: Data((canonical + "tampered").utf8)))
    }

    @Test
    func plaintextAPIRequestsFailBeforeEnrollment() async throws {
        let session = session()
        defer { session.invalidateAndCancel() }
        await #expect(throws: CowboyServiceClientError.invalidControllerURL) {
            try await DeviceProof().sign(URLRequest(url: URL(string: "http://127.0.0.1/api/auth/login")!), session: session)
        }
    }
}
