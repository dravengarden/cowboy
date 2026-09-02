import AppKit
import Combine
import CowboyInstallerCore
import SwiftUI

@main
enum CowboyManagerMain {
    @MainActor
    static func main() {
        let application = NSApplication.shared
        let delegate = AppDelegate()
        application.delegate = delegate
        application.setActivationPolicy(.accessory)
        application.run()
        withExtendedLifetime(delegate) {}
    }
}

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate, NSMenuDelegate, NSWindowDelegate {
    private let runtime = AppRuntime.live()
    private let navigation = ManagerNavigationModel()
    private var statusItem: NSStatusItem?
    private var managerWindow: NSWindow?
    private var subscriptions = Set<AnyCancellable>()

    func applicationDidFinishLaunching(_: Notification) {
        let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        item.button?.toolTip = "Cowboy Manager"
        let menu = NSMenu()
        menu.delegate = self
        menu.autoenablesItems = false
        item.menu = menu
        statusItem = item
        observeModel()
        updateStatusItem()
        if ProcessInfo.processInfo.arguments.contains("--show-manager") {
            DispatchQueue.main.async { [weak self] in self?.showManager(section: .dashboard) }
        } else if ProcessInfo.processInfo.arguments.contains("--show-settings") {
            DispatchQueue.main.async { [weak self] in self?.showManager(section: .settings) }
        }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_: NSApplication) -> Bool {
        false
    }

    func application(_: NSApplication, open urls: [URL]) {
        guard let callback = urls.first(where: {
            $0.scheme == "xyz.stormbird.cowboy.manager"
                && $0.host == "auth"
                && $0.path == "/callback"
        }) else {
            return
        }
        _ = runtime.model.completeFederatedSignIn(callbackURL: callback)
        showManager(section: .account)
    }

    func windowWillClose(_: Notification) {
        runtime.model.windowDidClose()
    }

    func menuNeedsUpdate(_ menu: NSMenu) {
        runtime.model.startMonitoring()
        rebuild(menu)
    }

    private func observeModel() {
        runtime.model.objectWillChange
            .receive(on: DispatchQueue.main)
            .sink { [weak self] _ in
                DispatchQueue.main.async { self?.updateStatusItem() }
            }
            .store(in: &subscriptions)
    }

    private func updateStatusItem() {
        guard let button = statusItem?.button else { return }
        let image = NSImage(
            systemSymbolName: runtime.model.menuBarSymbol,
            accessibilityDescription: runtime.model.menuBarMessage
        )
        image?.isTemplate = true
        button.image = image
        button.toolTip = runtime.model.menuBarMessage
    }

    private func rebuild(_ menu: NSMenu) {
        let model = runtime.model
        menu.removeAllItems()

        let status = NSMenuItem(title: model.menuBarMessage, action: nil, keyEquivalent: "")
        status.image = NSImage(
            systemSymbolName: model.menuBarSymbol,
            accessibilityDescription: model.menuBarMessage
        )
        status.isEnabled = false
        menu.addItem(status)
        menu.addItem(.separator())

        if model.controllerURL != nil {
            menu.addItem(item("Open Cowboy", action: #selector(openCowboy), key: "o"))
        }
        if model.installedStatus.isInstalled {
            if model.isMachineRunning {
                menu.addItem(item(
                    "Stop Cowboy Machine…",
                    action: #selector(stopMachine),
                    enabled: !model.serviceActionState.phase.isRunning
                ))
            } else {
                menu.addItem(item(
                    "Start Cowboy Machine",
                    action: #selector(startMachine),
                    enabled: !model.serviceActionState.phase.isRunning
                ))
            }
        } else {
            menu.addItem(item(
                "Install Cowboy Machine…",
                action: #selector(openInstall),
                enabled: !model.isRunning
            ))
        }
        menu.addItem(item(
            "Check Dependencies…",
            action: #selector(checkDependencies),
            enabled: model.accountStatus.canReadProduct
                && model.installedStatus.machineID != nil
                && !model.dependencyUpdateState.phase.isRunning
        ))

        menu.addItem(.separator())
        menu.addItem(item("Open Manager", action: #selector(openManager), key: "m"))
        menu.addItem(item("Account…", action: #selector(openAccount)))
        menu.addItem(item("Settings…", action: #selector(openSettings), key: ","))
        menu.addItem(.separator())
        menu.addItem(item(
            "Quit Cowboy Manager",
            action: #selector(quit),
            key: "q",
            enabled: !model.isBusy
        ))
    }

    private func item(
        _ title: String,
        action: Selector,
        key: String = "",
        enabled: Bool = true
    ) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: action, keyEquivalent: key)
        item.target = self
        item.isEnabled = enabled
        return item
    }

    @objc private func openCowboy() {
        CowboyOpener.open(controllerURL: runtime.model.controllerURL)
    }

    @objc private func startMachine() {
        runtime.model.startMachine()
    }

    @objc private func stopMachine() {
        NSApplication.shared.activate()
        let alert = NSAlert()
        alert.messageText = "Stop Cowboy Machine?"
        if let count = runtime.model.remoteMachine?.activeSessions, count > 0 {
            alert.informativeText = "\(count) active session\(count == 1 ? "" : "s") will disconnect until the Machine starts again."
        } else {
            alert.informativeText = "New sessions cannot run on this Mac until the Machine starts again."
        }
        alert.alertStyle = .warning
        alert.addButton(withTitle: "Stop Machine")
        alert.addButton(withTitle: "Cancel")
        if alert.runModal() == .alertFirstButtonReturn {
            runtime.model.stopMachine()
        }
    }

    @objc private func checkDependencies() {
        runtime.model.checkDependencies()
        showManager(section: .dashboard)
    }

    @objc private func openManager() {
        showManager(section: .dashboard)
    }

    @objc private func openInstall() {
        showManager(section: .install)
    }

    @objc private func openAccount() {
        showManager(section: .account)
    }

    @objc private func openSettings() {
        runtime.launchAtLogin.refresh()
        showManager(section: .settings)
    }

    @objc private func quit() {
        NSApplication.shared.terminate(nil)
    }

    private func showManager(section: ManagerSection) {
        runtime.model.startMonitoring()
        navigation.selection = section
        if managerWindow == nil {
            let root = ManagerRootView()
                .environmentObject(runtime.model)
                .environmentObject(runtime.launchAtLogin)
                .environmentObject(navigation)
            let window = NSWindow(contentViewController: NSHostingController(rootView: root))
            window.title = "Cowboy"
            window.styleMask = [.titled, .closable, .miniaturizable, .resizable]
            window.setContentSize(NSSize(width: 820, height: 560))
            window.minSize = NSSize(width: 720, height: 480)
            window.isReleasedWhenClosed = false
            window.delegate = self
            window.center()
            managerWindow = window
        }
        runtime.model.refreshAll()
        NSApplication.shared.activate()
        managerWindow?.makeKeyAndOrderFront(nil)
    }
}

extension AppModel {
    var menuBarColor: Color {
        if serviceActionState.phase == .failed {
            return .orange
        }
        if isMachineRunning {
            return .green
        }
        return installedStatus.isInstalled ? .secondary : .orange
    }
}

extension InstallState {
    var statusColor: Color {
        switch phase {
        case .succeeded: .green
        case .failed, .interrupted: .orange
        case .cancelled: .secondary
        case .preparing, .validating, .installing, .activating, .refreshing: .accentColor
        case .idle: .secondary
        }
    }
}
