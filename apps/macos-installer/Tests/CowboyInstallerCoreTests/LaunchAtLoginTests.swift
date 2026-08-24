import CowboyInstallerCore
import Foundation
import Testing

final class MockLoginItemService: LoginItemServicing {
    var status: LaunchAtLoginStatus = .disabled
    var registrationError: Error?
    var unregistrationError: Error?

    func register() throws {
        if let registrationError { throw registrationError }
        status = .enabled
    }

    func unregister() throws {
        if let unregistrationError { throw unregistrationError }
        status = .disabled
    }
}

@MainActor
struct LaunchAtLoginTests {
    @Test
    func reflectsSystemStatusAndChanges() {
        let service = MockLoginItemService()
        service.status = .requiresApproval
        let controller = LaunchAtLoginController(service: service)
        #expect(controller.status == .requiresApproval)

        service.status = .disabled
        controller.refresh()
        controller.setEnabled(true)

        #expect(controller.status == .enabled)
        #expect(controller.isEnabled)
    }

    @Test
    func preservesActualStatusWhenRegistrationFails() {
        let service = MockLoginItemService()
        service.registrationError = TestFailure(errorDescription: "registration denied")
        let controller = LaunchAtLoginController(service: service)

        controller.setEnabled(true)

        #expect(controller.status == .disabled)
        #expect(controller.errorMessage == "registration denied")
    }

    @Test
    func observesSystemRevocation() {
        let service = MockLoginItemService()
        service.status = .enabled
        let controller = LaunchAtLoginController(service: service)
        #expect(controller.isEnabled)

        service.status = .disabled
        controller.refresh()

        #expect(!controller.isEnabled)
    }
}
