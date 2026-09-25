import AppKit

/// Launch Services sends reopen when Finder/Dock activates an already running
/// app. SwiftUI owns window construction; the delegate only requests its single
/// workspace scene and never creates another model or desktop host.
@MainActor
public final class NativeApplicationDelegate: NSObject, NSApplicationDelegate {
    public var openWorkspace: (() -> Void)?
    private var reopenScheduled = false

    public func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }

    public func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        guard !flag, !reopenScheduled, openWorkspace != nil else { return true }
        reopenScheduled = true
        Task { @MainActor [weak self] in
            guard let self else { return }
            self.openWorkspace?()
            self.reopenScheduled = false
        }
        return true
    }
}
