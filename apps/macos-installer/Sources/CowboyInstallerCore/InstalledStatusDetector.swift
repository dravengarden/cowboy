@preconcurrency import Foundation
import Darwin

public protocol InstalledStatusDetecting: AnyObject {
    func detect(preferredStateDirectory: String?) -> InstalledStatus
}

public final class InstalledStatusDetector: InstalledStatusDetecting, @unchecked Sendable {
    private let fileManager: FileManager

    public init(fileManager: FileManager = .default) {
        self.fileManager = fileManager
    }

    public func detect(preferredStateDirectory: String?) -> InstalledStatus {
        let candidates = candidateDirectories(preferred: preferredStateDirectory)
        guard let stateDirectory = candidates.first(where: isInstalled(at:)) else {
            return .notInstalled
        }
        let originURL = stateDirectory.appendingPathComponent("service-origin")
        let origin = try? String(contentsOf: originURL, encoding: .utf8)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let version = executableVersion(at: stateDirectory.appendingPathComponent("bootstrap/cowboy-machine"))
        let serviceID = stateDirectory.lastPathComponent
        let launchAgent = fileManager.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/LaunchAgents/xyz.stormbird.cowboy-machine.\(serviceID).plist")
        return InstalledStatus(
            isInstalled: true,
            version: version,
            location: stateDirectory.path,
            serviceOrigin: origin,
            launchAgentLoaded: fileManager.fileExists(atPath: launchAgent.path)
                && isLaunchAgentLoaded(label: "xyz.stormbird.cowboy-machine.\(serviceID)")
        )
    }

    private func candidateDirectories(preferred: String?) -> [URL] {
        if let preferred, !preferred.isEmpty {
            return [URL(fileURLWithPath: preferred, isDirectory: true)]
        }
        let services = fileManager.homeDirectoryForCurrentUser
            .appendingPathComponent(".local/state/cowboy-machine/services", isDirectory: true)
        return (try? fileManager.contentsOfDirectory(
            at: services,
            includingPropertiesForKeys: [.isDirectoryKey],
            options: [.skipsHiddenFiles]
        ))?.sorted { $0.lastPathComponent < $1.lastPathComponent } ?? []
    }

    private func isInstalled(at directory: URL) -> Bool {
        fileManager.isExecutableFile(
            atPath: directory.appendingPathComponent("bootstrap/cowboy-machine").path
        ) && fileManager.fileExists(
            atPath: directory.appendingPathComponent("service-origin").path
        )
    }

    private func executableVersion(at executable: URL) -> String? {
        guard fileManager.isExecutableFile(atPath: executable.path) else { return nil }
        let process = Process()
        process.executableURL = executable
        process.arguments = ["--version"]
        let output = Pipe()
        process.standardOutput = output
        process.standardError = FileHandle.nullDevice
        do {
            try process.run()
            process.waitUntilExit()
        } catch {
            return nil
        }
        guard process.terminationStatus == 0 else { return nil }
        let text = String(data: output.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8)?
        return text.trimmingCharacters(in: .whitespacesAndNewlines)
            .split(separator: " ").last.map(String.init)
    }

    private func isLaunchAgentLoaded(label: String) -> Bool {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/bin/launchctl")
        process.arguments = ["print", "gui/\(getuid())/\(label)"]
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice
        do {
            try process.run()
            process.waitUntilExit()
            return process.terminationStatus == 0
        } catch {
            return false
        }
    }
}
