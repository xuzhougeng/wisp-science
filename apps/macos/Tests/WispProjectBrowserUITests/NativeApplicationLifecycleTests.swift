import AppKit
import XCTest
@testable import WispProjectBrowserUI

final class NativeApplicationLifecycleTests: XCTestCase {
    @MainActor func testReopenRestoresMissingWorkspaceOnceAndDoesNotReplaceVisibleWorkspace() async {
        let delegate = NativeApplicationDelegate()
        var opens = 0
        delegate.openWorkspace = { opens += 1 }
        let app = NSApplication.shared
        XCTAssertFalse(delegate.applicationShouldTerminateAfterLastWindowClosed(app))
        XCTAssertTrue(delegate.applicationShouldHandleReopen(app, hasVisibleWindows: true))
        await Task.yield()
        XCTAssertEqual(opens, 0)
        XCTAssertTrue(delegate.applicationShouldHandleReopen(app, hasVisibleWindows: false))
        XCTAssertTrue(delegate.applicationShouldHandleReopen(app, hasVisibleWindows: false))
        for _ in 0..<10 { await Task.yield() }
        XCTAssertEqual(opens, 1)
        XCTAssertTrue(delegate.applicationShouldHandleReopen(app, hasVisibleWindows: false))
        for _ in 0..<10 { await Task.yield() }
        XCTAssertEqual(opens, 2)
    }
}
