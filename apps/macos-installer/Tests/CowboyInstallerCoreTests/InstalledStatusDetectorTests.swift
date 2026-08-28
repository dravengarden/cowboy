import CowboyInstallerCore
import Foundation
import Testing

struct InstalledStatusDetectorTests {
    @Test
    func detectsTheLegacyMacInstallationAndControllerOrigin() throws {
        let fileManager = FileManager.default
        let home = fileManager.temporaryDirectory
            .appendingPathComponent("CowboyInstalledStatusTests.\(UUID().uuidString)", isDirectory: true)
        defer { try? fileManager.removeItem(at: home) }
        let state = home.appendingPathComponent(".local/state/cowboy-machine", isDirectory: true)
        let bootstrap = state.appendingPathComponent("bootstrap/cowboy-machine")
        let launcher = home.appendingPathComponent(".local/bin/cowboy-machine-launch")
        let launchAgents = home.appendingPathComponent("Library/LaunchAgents", isDirectory: true)
        let plist = launchAgents.appendingPathComponent("xyz.stormbird.cowboy-machine.plist")
        try fileManager.createDirectory(
            at: bootstrap.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        try fileManager.createDirectory(at: launcher.deletingLastPathComponent(), withIntermediateDirectories: true)
        try fileManager.createDirectory(at: launchAgents, withIntermediateDirectories: true)
        try "binary".write(to: bootstrap, atomically: true, encoding: .utf8)
        try fileManager.setAttributes([.posixPermissions: 0o700], ofItemAtPath: bootstrap.path)
        try "identity".write(
            to: state.appendingPathComponent("identity_ed25519"),
            atomically: true,
            encoding: .utf8
        )
        try "macbook-air\n".write(
            to: state.appendingPathComponent("machine-id"),
            atomically: true,
            encoding: .utf8
        )
        try "exec machine '--controller-url' 'https://cowboy.example' '--state-dir' '\(state.path)'\n"
            .write(to: launcher, atomically: true, encoding: .utf8)
        let plistText = """
        <?xml version="1.0" encoding="UTF-8"?>
        <plist version="1.0"><dict>
        <key>Label</key><string>xyz.stormbird.cowboy-machine</string>
        <key>ProgramArguments</key><array><string>\(launcher.path)</string></array>
        </dict></plist>
        """
        try plistText.write(to: plist, atomically: true, encoding: .utf8)
        let detector = InstalledStatusDetector(
            homeDirectory: home,
            launchAgentProbe: { $0 == "xyz.stormbird.cowboy-machine" },
            versionProbe: { _ in "0.1.0" }
        )

        let status = detector.detect(preferredStateDirectory: nil)

        #expect(status.isInstalled)
        #expect(status.location == state.path)
        #expect(status.serviceOrigin == "https://cowboy.example")
        #expect(status.machineID == "macbook-air")
        #expect(status.launchAgentLabel == "xyz.stormbird.cowboy-machine")
        #expect(status.launchAgentPath == plist.path)
        #expect(status.launchAgentLoaded)
        #expect(status.version == "0.1.0")
    }
}
