import Foundation
import CryptoKit
import Security

/// One local key per HTTPS origin. Account cookies remain in URLSession;
/// neither a cookie alone nor this key alone authorizes a Service request.
actor DeviceProof {
    private struct Challenge: Decodable {
        let epoch: String
        let server_time_ms: Int64
    }
    private struct Proof: Encodable {
        let key: String
        let epoch: String
        let origin: String
        let time: Int64
        let nonce: String
        let signature: String
    }
    private var keys: [String: P256.Signing.PrivateKey] = [:]
    private var challenges: [String: (epoch: String, offset: Int64)] = [:]
    private let keyLoader: @Sendable (String) throws -> P256.Signing.PrivateKey

    init(keyLoader: @escaping @Sendable (String) throws -> P256.Signing.PrivateKey = DeviceProof.keychainIdentity) {
        self.keyLoader = keyLoader
    }

    func sign(_ request: URLRequest, session: URLSession, refresh: Bool = false) async throws -> URLRequest {
        guard let url = request.url, url.path.hasPrefix("/api/") else { return request }
        guard url.scheme == "https", var parts = URLComponents(url: url, resolvingAgainstBaseURL: false) else {
            throw CowboyServiceClientError.invalidControllerURL
        }
        parts.path = ""
        parts.host = parts.host?.lowercased()
        if parts.port == 443 { parts.port = nil }
        parts.query = nil
        parts.fragment = nil
        guard let origin = parts.url?.absoluteString.trimmingCharacters(in: CharacterSet(charactersIn: "/")) else {
            throw CowboyServiceClientError.invalidControllerURL
        }
        let key = try identity(origin)
        if refresh { challenges.removeValue(forKey: origin) }
        if challenges[origin] == nil {
            parts.path = "/api/auth/browser/challenge"
            var challengeRequest = URLRequest(url: parts.url!)
            challengeRequest.httpShouldHandleCookies = false
            challengeRequest.cachePolicy = .reloadIgnoringLocalCacheData
            let (data, response) = try await session.data(for: challengeRequest)
            guard (response as? HTTPURLResponse)?.statusCode == 200 else {
                throw CowboyServiceClientError.invalidResponse
            }
            let challenge = try JSONDecoder().decode(Challenge.self, from: data)
            challenges[origin] = (challenge.epoch, challenge.server_time_ms - now())
        }
        guard let challenge = challenges[origin] else { throw CowboyServiceClientError.invalidResponse }
        var nonce = [UInt8](repeating: 0, count: 32)
        guard SecRandomCopyBytes(kSecRandomDefault, nonce.count, &nonce) == errSecSuccess else {
            throw CowboyServiceClientError.secureRandomnessUnavailable
        }
        let publicKey = base64(key.publicKey.x963Representation)
        let timestamp = now() + challenge.offset
        let random = base64(Data(nonce))
        let target = url.path(percentEncoded: true) + (url.query.map { "?" + $0 } ?? "")
        let message = ["cowboy-browser-proof-v1", challenge.epoch, origin, publicKey,
                       request.httpMethod ?? "GET", target, String(timestamp), random].joined(separator: "\n")
        let signature = try key.signature(for: Data(message.utf8))
        let proof = Proof(key: publicKey, epoch: challenge.epoch, origin: origin,
                          time: timestamp, nonce: random, signature: base64(signature.rawRepresentation))
        var signed = request
        signed.setValue(base64(try JSONEncoder().encode(proof)), forHTTPHeaderField: "x-cowboy-browser-proof")
        return signed
    }

    private func identity(_ origin: String) throws -> P256.Signing.PrivateKey {
        if let key = keys[origin] { return key }
        let key = try keyLoader(origin)
        keys[origin] = key
        return key
    }

    private static func keychainIdentity(_ origin: String) throws -> P256.Signing.PrivateKey {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: "xyz.stormbird.cowboy.manager.device",
            kSecAttrAccount as String: origin,
        ]
        var lookup = query
        lookup[kSecReturnData as String] = true
        lookup[kSecMatchLimit as String] = kSecMatchLimitOne
        var stored: CFTypeRef?
        let status = SecItemCopyMatching(lookup as CFDictionary, &stored)
        if status == errSecSuccess, let data = stored as? Data {
            return try P256.Signing.PrivateKey(rawRepresentation: data)
        }
        guard status == errSecItemNotFound else { throw ServiceCredentialStoreError.keychain(status) }
        let key = P256.Signing.PrivateKey()
        var insert = query
        insert[kSecValueData as String] = key.rawRepresentation
        insert[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        let added = SecItemAdd(insert as CFDictionary, nil)
        if added == errSecDuplicateItem { return try keychainIdentity(origin) }
        guard added == errSecSuccess else { throw ServiceCredentialStoreError.keychain(added) }
        return key
    }

    private func now() -> Int64 { Int64(Date().timeIntervalSince1970 * 1000) }
    private func base64(_ data: Data) -> String {
        data.base64EncodedString().replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_").replacingOccurrences(of: "=", with: "")
    }
}
