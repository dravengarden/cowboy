import AppKit

enum CowboyOpener {
    private static let desktopBundleIdentifier = "top.thundersparrow.cowboy"

    static func open(controllerURL: URL?) {
        guard let applicationURL = NSWorkspace.shared.urlForApplication(
            withBundleIdentifier: desktopBundleIdentifier
        ) else {
            if let controllerURL {
                NSWorkspace.shared.open(controllerURL)
            }
            return
        }
        let configuration = NSWorkspace.OpenConfiguration()
        configuration.activates = true
        NSWorkspace.shared.openApplication(
            at: applicationURL,
            configuration: configuration
        ) { _, error in
            if error != nil, let controllerURL {
                NSWorkspace.shared.open(controllerURL)
            }
        }
    }
}
