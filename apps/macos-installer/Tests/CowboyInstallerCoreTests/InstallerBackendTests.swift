import CowboyInstallerCore
import Foundation
import Testing

@MainActor
struct InstallerBackendTests {
    @Test
    func enrollmentTokenUsesAnonymousPipeInsteadOfArguments() async throws {
        let fileManager = FileManager.default
        let directory = fileManager.temporaryDirectory
            .appendingPathComponent("CowboyInstallerBackendTests.\(UUID().uuidString)")
        try fileManager.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? fileManager.removeItem(at: directory) }
        let argumentsURL = directory.appendingPathComponent("arguments")
        let executableURL = directory.appendingPathComponent("cowboy")
        let script = """
        #!/bin/sh
        printf '%s\\n' "$@" > '\(argumentsURL.path)'
        token_file=''
        previous=''
        for argument in "$@"; do
            if [ "$previous" = '--token-file' ]; then token_file="$argument"; fi
            previous="$argument"
        done
        test "$(cat "$token_file")" = 'super-secret-token'
        """
        try script.write(to: executableURL, atomically: true, encoding: .utf8)
        try fileManager.setAttributes([.posixPermissions: 0o700], ofItemAtPath: executableURL.path)
        let backend = ProcessInstallerBackend(executableLocator: { executableURL })

        _ = try await backend.install(request: testRequest(token: "super-secret-token")) { _ in }

        let arguments = try String(contentsOf: argumentsURL, encoding: .utf8)
        #expect(arguments.contains("/dev/fd/0"))
        #expect(!arguments.contains("super-secret-token"))
    }
}
