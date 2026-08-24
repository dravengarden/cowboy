@preconcurrency import Foundation

public protocol InstallerBackend: AnyObject {
    @MainActor
    func install(
        request: InstallRequest,
        onProgress: @escaping (InstallProgress) -> Void
    ) async throws -> InstallReceipt

    @MainActor
    func cancel()
}

public enum InstallerBackendError: LocalizedError, Equatable {
    case invalidControllerURL
    case enrollmentTokenRequired
    case workspaceNotFound(String)
    case stateDirectoryNotAbsolute
    case bundledInstallerMissing(String)
    case installerFailed(String)
    case verificationFailed(String)

    public var errorDescription: String? {
        switch self {
        case .invalidControllerURL:
            "Enter a valid HTTPS Cowboy Service URL. Loopback HTTP is allowed for development."
        case .enrollmentTokenRequired:
            "Create a one-time Machine enrollment code in Cowboy, then paste it here."
        case let .workspaceNotFound(path):
            "The workspace directory does not exist: \(path)"
        case .stateDirectoryNotAbsolute:
            "The custom installation directory must be an absolute path."
        case let .bundledInstallerMissing(path):
            "The bundled Cowboy installer is missing or not executable: \(path)"
        case let .installerFailed(message):
            message
        case let .verificationFailed(message):
            message
        }
    }
}

@MainActor
public final class ProcessInstallerBackend: InstallerBackend {
    public typealias ExecutableLocator = () -> URL?

    private let executableLocator: ExecutableLocator
    private let fileManager: FileManager
    private var process: Process?
    private var cancellationRequested = false

    public init(fileManager: FileManager = .default) {
        executableLocator = {
            Bundle.main.resourceURL?
                .appendingPathComponent("bin", isDirectory: true)
                .appendingPathComponent("cowboy")
        }
        self.fileManager = fileManager
    }

    public init(executableLocator: @escaping ExecutableLocator, fileManager: FileManager = .default) {
        self.executableLocator = executableLocator
        self.fileManager = fileManager
    }

    public func install(
        request: InstallRequest,
        onProgress: @escaping (InstallProgress) -> Void
    ) async throws -> InstallReceipt {
        try validate(request)
        guard let executableURL = executableLocator(), fileManager.isExecutableFile(atPath: executableURL.path) else {
            throw InstallerBackendError.bundledInstallerMissing(
                executableLocator()?.path ?? "Contents/Resources/bin/cowboy"
            )
        }

        cancellationRequested = false
        onProgress(.init(phase: .preparing, fraction: 0.08, message: "Preparing a secure enrollment request"))
        let tokenURL = try writeTemporaryToken(request.enrollmentToken)
        defer { try? fileManager.removeItem(at: tokenURL.deletingLastPathComponent()) }

        var arguments = [
            "register",
            request.controllerURL.trimmingCharacters(in: .whitespacesAndNewlines),
            "--workspace",
            "home=\(request.workspaceDirectory)",
            "--token-file",
            tokenURL.path,
            "--background",
        ]
        if let stateDirectory = request.stateDirectory, !stateDirectory.isEmpty {
            arguments.append(contentsOf: ["--state-dir", stateDirectory])
        }

        onProgress(.init(phase: .validating, fraction: 0.2, message: "Validating the Cowboy Service and local paths"))
        let child = Process()
        child.executableURL = executableURL
        child.arguments = arguments
        let outputPipe = Pipe()
        let errorPipe = Pipe()
        child.standardOutput = outputPipe
        child.standardError = errorPipe
        process = child

        onProgress(.init(phase: .installing, fraction: 0.48, message: "Installing Cowboy Machine with the existing backend"))
        let status = try await run(child)
        process = nil

        let standardOutput = String(
            data: outputPipe.fileHandleForReading.readDataToEndOfFile(),
            encoding: .utf8
        ) ?? ""
        let standardError = String(
            data: errorPipe.fileHandleForReading.readDataToEndOfFile(),
            encoding: .utf8
        ) ?? ""
        let log = Self.safeLog(stdout: standardOutput, stderr: standardError)

        if cancellationRequested || Task.isCancelled {
            throw CancellationError()
        }
        guard status == 0 else {
            throw InstallerBackendError.installerFailed(
                Self.userFacingFailure(from: standardError, status: status)
            )
        }

        onProgress(.init(phase: .activating, fraction: 0.82, message: "Verifying the background LaunchAgent"))
        let directory = request.stateDirectory?.isEmpty == false
            ? request.stateDirectory!
            : fileManager.homeDirectoryForCurrentUser
                .appendingPathComponent(".local/state/cowboy-machine/services", isDirectory: true).path
        return InstallReceipt(
            installedVersion: request.targetVersion,
            installationDirectory: directory,
            log: log.isEmpty ? "Cowboy Machine installed and started." : log
        )
    }

    public func cancel() {
        cancellationRequested = true
        process?.terminate()
    }

    private func validate(_ request: InstallRequest) throws {
        guard let components = URLComponents(string: request.controllerURL),
              let scheme = components.scheme?.lowercased(),
              let host = components.host,
              scheme == "https" || (scheme == "http" && ["localhost", "127.0.0.1", "::1"].contains(host))
        else {
            throw InstallerBackendError.invalidControllerURL
        }
        guard !request.enrollmentToken.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw InstallerBackendError.enrollmentTokenRequired
        }
        var isDirectory: ObjCBool = false
        guard fileManager.fileExists(atPath: request.workspaceDirectory, isDirectory: &isDirectory), isDirectory.boolValue else {
            throw InstallerBackendError.workspaceNotFound(request.workspaceDirectory)
        }
        if let stateDirectory = request.stateDirectory, !stateDirectory.isEmpty, !stateDirectory.hasPrefix("/") {
            throw InstallerBackendError.stateDirectoryNotAbsolute
        }
    }

    private func writeTemporaryToken(_ token: String) throws -> URL {
        let directory = fileManager.temporaryDirectory
            .appendingPathComponent("cowboy-installer-\(UUID().uuidString)", isDirectory: true)
        try fileManager.createDirectory(at: directory, withIntermediateDirectories: true)
        let tokenURL = directory.appendingPathComponent("enrollment-token")
        try Data(token.utf8).write(to: tokenURL, options: .atomic)
        try fileManager.setAttributes([.posixPermissions: 0o600], ofItemAtPath: tokenURL.path)
        return tokenURL
    }

    private func run(_ child: Process) async throws -> Int32 {
        try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                child.terminationHandler = { process in
                    continuation.resume(returning: process.terminationStatus)
                }
                do {
                    try child.run()
                } catch {
                    continuation.resume(throwing: error)
                }
            }
        } onCancel: {
            child.terminate()
        }
    }

    private static func safeLog(stdout: String, stderr: String) -> String {
        [stdout, stderr]
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
            .joined(separator: "\n")
    }

    private static func userFacingFailure(from stderr: String, status: Int32) -> String {
        let trimmed = stderr.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.isEmpty {
            return "Cowboy installer exited with status \(status)."
        }
        return String(trimmed.suffix(2_000))
    }
}
