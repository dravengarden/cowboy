import CowboyInstallerCore
import Foundation
import Testing

struct PersistenceTests {
    @Test
    func settingsPersistAcrossStoreInstances() {
        let suite = "CowboyInstallerTests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite) }
        let first = UserDefaultsInstallerPersistence(defaults: defaults)
        let expected = InstallerSettings(
            controllerURL: "https://cowboy.example",
            workspaceDirectory: "/Users/test/work",
            stateDirectory: "/Users/test/state",
            automaticallyCheckForUpdates: false,
            notificationsEnabled: false
        )

        first.saveSettings(expected)
        let second = UserDefaultsInstallerPersistence(defaults: defaults)

        #expect(second.loadSettings() == expected)
    }
}
