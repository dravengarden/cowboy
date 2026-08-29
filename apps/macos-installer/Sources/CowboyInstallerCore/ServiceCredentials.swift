import Foundation
import Security

public struct ServiceCredential: Codable, Equatable, Sendable {
    public let account: String
    public let password: String

    public init(account: String, password: String) {
        self.account = account
        self.password = password
    }
}

public protocol ServiceCredentialStoring: AnyObject {
    func load(controllerURL: String) throws -> ServiceCredential?
    func save(_ credential: ServiceCredential, controllerURL: String) throws
    func delete(controllerURL: String) throws
}

public enum ServiceCredentialStoreError: LocalizedError, Equatable {
    case invalidControllerURL
    case invalidCredential
    case keychain(OSStatus)

    public var errorDescription: String? {
        switch self {
        case .invalidControllerURL:
            "Enter a valid Cowboy Service URL before saving automatic sign-in."
        case .invalidCredential:
            "The saved Cowboy sign-in could not be read. Sign in again to replace it."
        case let .keychain(status):
            "macOS Keychain could not update automatic sign-in (status \(status))."
        }
    }
}

public final class KeychainServiceCredentialStore: ServiceCredentialStoring, @unchecked Sendable {
    private let service = "xyz.stormbird.cowboy.manager.login"
    private let encoder = JSONEncoder()
    private let decoder = JSONDecoder()

    public init() {}

    public func load(controllerURL: String) throws -> ServiceCredential? {
        let account = try credentialAccount(controllerURL)
        var result: CFTypeRef?
        let status = SecItemCopyMatching([
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecMatchLimit as String: kSecMatchLimitOne,
            kSecReturnData as String: true,
        ] as CFDictionary, &result)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess else {
            throw ServiceCredentialStoreError.keychain(status)
        }
        guard let data = result as? Data,
              let credential = try? decoder.decode(ServiceCredential.self, from: data),
              !credential.account.isEmpty,
              !credential.password.isEmpty
        else {
            throw ServiceCredentialStoreError.invalidCredential
        }
        return credential
    }

    public func save(_ credential: ServiceCredential, controllerURL: String) throws {
        guard !credential.account.isEmpty, !credential.password.isEmpty else {
            throw ServiceCredentialStoreError.invalidCredential
        }
        let account = try credentialAccount(controllerURL)
        let data = try encoder.encode(credential)
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        let update: [String: Any] = [
            kSecValueData as String: data,
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
        ]
        let status = SecItemUpdate(query as CFDictionary, update as CFDictionary)
        if status == errSecItemNotFound {
            var item = query
            item.merge(update) { _, latest in latest }
            let addStatus = SecItemAdd(item as CFDictionary, nil)
            guard addStatus == errSecSuccess else {
                throw ServiceCredentialStoreError.keychain(addStatus)
            }
            return
        }
        guard status == errSecSuccess else {
            throw ServiceCredentialStoreError.keychain(status)
        }
    }

    public func delete(controllerURL: String) throws {
        let account = try credentialAccount(controllerURL)
        let status = SecItemDelete([
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ] as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw ServiceCredentialStoreError.keychain(status)
        }
    }

    private func credentialAccount(_ controllerURL: String) throws -> String {
        let trimmed = controllerURL.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let components = URLComponents(string: trimmed),
              let scheme = components.scheme?.lowercased(),
              let host = components.host?.lowercased(),
              components.user == nil,
              components.password == nil,
              components.query == nil,
              components.fragment == nil,
              scheme == "https" || (scheme == "http" && ["localhost", "127.0.0.1", "::1"].contains(host))
        else {
            throw ServiceCredentialStoreError.invalidControllerURL
        }
        let formattedHost = host.contains(":") ? "[\(host)]" : host
        if let port = components.port {
            return "\(scheme)://\(formattedHost):\(port)"
        }
        return "\(scheme)://\(formattedHost)"
    }
}
