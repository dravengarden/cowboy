import Foundation

public protocol InstallerPersistence: AnyObject {
    func loadSettings() -> InstallerSettings
    func saveSettings(_ settings: InstallerSettings)
    func loadHistory() -> [ActivityRecord]
    func saveHistory(_ history: [ActivityRecord])
    func loadPendingInstall() -> PendingInstall?
    func savePendingInstall(_ pending: PendingInstall?)
}

public final class UserDefaultsInstallerPersistence: InstallerPersistence, @unchecked Sendable {
    private enum Key {
        static let settings = "installer.settings.v1"
        static let history = "installer.history.v1"
        static let pending = "installer.pending.v1"
    }

    private let defaults: UserDefaults
    private let encoder = JSONEncoder()
    private let decoder = JSONDecoder()

    public init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
    }

    public func loadSettings() -> InstallerSettings {
        decode(InstallerSettings.self, forKey: Key.settings) ?? InstallerSettings()
    }

    public func saveSettings(_ settings: InstallerSettings) {
        encode(settings, forKey: Key.settings)
    }

    public func loadHistory() -> [ActivityRecord] {
        decode([ActivityRecord].self, forKey: Key.history) ?? []
    }

    public func saveHistory(_ history: [ActivityRecord]) {
        encode(history, forKey: Key.history)
    }

    public func loadPendingInstall() -> PendingInstall? {
        decode(PendingInstall.self, forKey: Key.pending)
    }

    public func savePendingInstall(_ pending: PendingInstall?) {
        guard let pending else {
            defaults.removeObject(forKey: Key.pending)
            return
        }
        encode(pending, forKey: Key.pending)
    }

    private func encode<T: Encodable>(_ value: T, forKey key: String) {
        if let data = try? encoder.encode(value) {
            defaults.set(data, forKey: key)
        }
    }

    private func decode<T: Decodable>(_ type: T.Type, forKey key: String) -> T? {
        guard let data = defaults.data(forKey: key) else { return nil }
        return try? decoder.decode(type, from: data)
    }
}
