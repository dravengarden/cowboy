@preconcurrency import Foundation
import Darwin

public protocol InstalledStatusDetecting: AnyObject {
    func detect(preferredStateDirectory: String?) -> InstalledStatus
}

public final class InstalledStatusDetector: InstalledStatusDetecting, @unchecked Sendable {
    public typealias LaunchAgentProbe = (String) -> Bool
    public typealias VersionProbe = (URL) -> String?

    private static let launchAgentPrefix = "xyz.stormbird.cowboy-machine"

    private let fileManager: FileManager
    private let homeDirectory: URL
    private let launchAgentProbe: LaunchAgentProbe
    private let versionProbe: VersionProbe

    public init(
        fileManager: FileManager = .default,
        homeDirectory: URL? = nil,
        launchAgentProbe: LaunchAgentProbe? = nil,
        versionProbe: VersionProbe? = nil
    ) {
        self.fileManager = fileManager
        self.homeDirectory = homeDirectory ?? fileManager.homeDirectoryForCurrentUser
        self.launchAgentProbe = launchAgentProbe ?? Self.isLaunchAgentLoaded
        self.versionProbe = versionProbe ?? Self.executableVersion
    }

    public func detect(preferredStateDirectory: String?) -> InstalledStatus {
        let candidates = candidateDirectories(preferred: preferredStateDirectory)
        guard let stateDirectory = candidates.first(where: isInstalled(at:)) else {
            return .notInstalled
        }
        let launchAgent = launchAgent(for: stateDirectory)
        let origin = text(at: stateDirectory.appendingPathComponent("service-origin"))
            ?? launchAgent.flatMap { controllerOrigin(fromLauncherAt: $0.launcher) }
        let activeMachine = stateDirectory.appendingPathComponent("components/commands/cowboy-machine")
        let bootstrapMachine = stateDirectory.appendingPathComponent("bootstrap/cowboy-machine")
        let version = versionProbe(
            fileManager.isExecutableFile(atPath: activeMachine.path) ? activeMachine : bootstrapMachine
        )
        let machineID = text(at: stateDirectory.appendingPathComponent("machine-id"))
        let serviceID = launchAgent?.serviceID ?? inferredServiceID(from: stateDirectory)
        return InstalledStatus(
            isInstalled: true,
            version: version,
            location: stateDirectory.path,
            serviceOrigin: origin,
            serviceID: serviceID,
            machineID: machineID,
            launchAgentLabel: launchAgent?.label,
            launchAgentPath: launchAgent?.plist.path,
            launchAgentLoaded: launchAgent.map { launchAgentProbe($0.label) } ?? false
        )
    }

    private func candidateDirectories(preferred: String?) -> [URL] {
        var candidates: [URL] = []
        if let preferred, !preferred.isEmpty {
            candidates.append(URL(fileURLWithPath: preferred, isDirectory: true))
        }
        let root = homeDirectory
            .appendingPathComponent(".local/state/cowboy-machine", isDirectory: true)
        let services = root.appendingPathComponent("services", isDirectory: true)
        let serviceDirectories = (try? fileManager.contentsOfDirectory(
            at: services,
            includingPropertiesForKeys: [.isDirectoryKey],
            options: [.skipsHiddenFiles]
        ))?.sorted { $0.lastPathComponent < $1.lastPathComponent } ?? []
        candidates.append(contentsOf: serviceDirectories)
        candidates.append(root)

        var seen = Set<String>()
        return candidates.filter { seen.insert($0.standardizedFileURL.path).inserted }
    }

    private func isInstalled(at directory: URL) -> Bool {
        let bootstrap = directory.appendingPathComponent("bootstrap/cowboy-machine")
        let active = directory.appendingPathComponent("components/commands/cowboy-machine")
        guard fileManager.isExecutableFile(atPath: bootstrap.path)
            || fileManager.isExecutableFile(atPath: active.path)
        else {
            return false
        }
        return fileManager.fileExists(atPath: directory.appendingPathComponent("identity_ed25519").path)
            || fileManager.fileExists(atPath: directory.appendingPathComponent("service-origin").path)
            || launchAgent(for: directory) != nil
    }

    private func launchAgent(for stateDirectory: URL) -> LaunchAgentDescriptor? {
        let launchAgents = homeDirectory
            .appendingPathComponent("Library/LaunchAgents", isDirectory: true)
        let inferredServiceID = inferredServiceID(from: stateDirectory)
        let preferredLabels = [
            inferredServiceID.map { "\(Self.launchAgentPrefix).\($0)" },
            Self.launchAgentPrefix,
        ].compactMap { $0 }
        for label in preferredLabels {
            let plist = launchAgents.appendingPathComponent("\(label).plist")
            if let launcher = launcherPath(from: plist), launcherReferences(launcher, stateDirectory: stateDirectory) {
                return LaunchAgentDescriptor(
                    label: label,
                    plist: plist,
                    launcher: launcher,
                    serviceID: serviceID(from: label)
                )
            }
        }

        let plists = (try? fileManager.contentsOfDirectory(
            at: launchAgents,
            includingPropertiesForKeys: nil,
            options: [.skipsHiddenFiles]
        )) ?? []
        for plist in plists where plist.lastPathComponent.hasPrefix(Self.launchAgentPrefix)
            && plist.pathExtension == "plist"
        {
            guard let launcher = launcherPath(from: plist),
                  launcherReferences(launcher, stateDirectory: stateDirectory)
            else {
                continue
            }
            let label = plist.deletingPathExtension().lastPathComponent
            return LaunchAgentDescriptor(
                label: label,
                plist: plist,
                launcher: launcher,
                serviceID: serviceID(from: label)
            )
        }
        return nil
    }

    private func launcherPath(from plist: URL) -> URL? {
        guard let data = try? Data(contentsOf: plist),
              let value = try? PropertyListSerialization.propertyList(from: data, format: nil),
              let dictionary = value as? [String: Any],
              let arguments = dictionary["ProgramArguments"] as? [String],
              let launcher = arguments.first,
              launcher.hasPrefix("/")
        else {
            return nil
        }
        return URL(fileURLWithPath: launcher)
    }

    private func launcherReferences(_ launcher: URL, stateDirectory: URL) -> Bool {
        guard let script = try? String(contentsOf: launcher, encoding: .utf8) else { return false }
        return script.contains(stateDirectory.standardizedFileURL.path)
    }

    private func controllerOrigin(fromLauncherAt launcher: URL) -> String? {
        guard let script = try? String(contentsOf: launcher, encoding: .utf8),
              let expression = try? NSRegularExpression(
                  pattern: #"(?:'|\")?--controller-url(?:'|\")?\s+(?:'|\")([^'\"]+)(?:'|\")"#
              )
        else {
            return nil
        }
        let range = NSRange(script.startIndex..<script.endIndex, in: script)
        guard let match = expression.firstMatch(in: script, range: range),
              let valueRange = Range(match.range(at: 1), in: script)
        else {
            return nil
        }
        return String(script[valueRange])
    }

    private func inferredServiceID(from stateDirectory: URL) -> String? {
        guard stateDirectory.deletingLastPathComponent().lastPathComponent == "services" else {
            return nil
        }
        let candidate = stateDirectory.lastPathComponent
        return candidate.hasPrefix("svc-") ? candidate : nil
    }

    private func serviceID(from label: String) -> String? {
        let prefix = "\(Self.launchAgentPrefix)."
        guard label.hasPrefix(prefix) else { return nil }
        let value = String(label.dropFirst(prefix.count))
        return value.isEmpty ? nil : value
    }

    private func text(at url: URL) -> String? {
        guard let value = try? String(contentsOf: url, encoding: .utf8)
            .trimmingCharacters(in: .whitespacesAndNewlines),
            !value.isEmpty
        else {
            return nil
        }
        return value
    }

    private static func executableVersion(at executable: URL) -> String? {
        guard FileManager.default.isExecutableFile(atPath: executable.path) else { return nil }
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
        guard process.terminationStatus == 0,
              let text = String(
                  data: output.fileHandleForReading.readDataToEndOfFile(),
                  encoding: .utf8
              )
        else {
            return nil
        }
        return text.trimmingCharacters(in: .whitespacesAndNewlines)
            .split(separator: " ").last.map(String.init)
    }

    private static func isLaunchAgentLoaded(label: String) -> Bool {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/bin/launchctl")
        process.arguments = ["print", "gui/\(getuid())/\(label)"]
        let output = Pipe()
        process.standardOutput = output
        process.standardError = FileHandle.nullDevice
        do {
            try process.run()
            process.waitUntilExit()
            guard process.terminationStatus == 0 else { return false }
            let text = String(
                data: output.fileHandleForReading.readDataToEndOfFile(),
                encoding: .utf8
            ) ?? ""
            return text.contains("state = running") || text.range(
                of: #"\bpid\s*=\s*\d+"#,
                options: .regularExpression
            ) != nil
        } catch {
            return false
        }
    }
}

private struct LaunchAgentDescriptor {
    let label: String
    let plist: URL
    let launcher: URL
    let serviceID: String?
}
