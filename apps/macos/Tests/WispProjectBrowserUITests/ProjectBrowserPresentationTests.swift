import AppKit
import XCTest
@testable import WispProjectBrowser
@testable import WispProjectBrowserUI

final class ProjectBrowserPresentationTests: XCTestCase {
    private func projects() throws -> [ProjectSummary] {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        let data = try Data(contentsOf: root.appendingPathComponent("contracts/project-browser/v1/projects.json"))
        let first = try ProjectBrowserClient.decode(data, requestID: "projects-1").projects[0]
        var second = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(first)) as? [String: Any])
        second["id"] = "second"
        second["name"] = "Protein analysis"
        second["description"] = "差异蛋白质分析"
        second["starred"] = false
        // Deliberately share a directory to check selection is keyed by project ID.
        let project = try JSONDecoder().decode(ProjectSummary.self, from: JSONSerialization.data(withJSONObject: second))
        return [first, project]
    }

    func testSearchMatchesNameDescriptionAndPathAndClearsHiddenSelection() throws {
        let projects = try projects()
        var state = ProjectBrowserPresentation()
        state.reconcile(projects)
        XCTAssertEqual(state.selectedID, projects[0].id)
        for query in ["protein", "差异蛋白", "  PROTEIN  "] {
            state.search = query
            state.reconcile(projects)
            XCTAssertEqual(state.visibleProjects(projects).map(\.id), ["second"])
            XCTAssertEqual(state.selectedID, "second")
        }
        state.search = "/Users/researcher"
        XCTAssertEqual(state.visibleProjects(projects).count, 2)
        state.search = "missing project"
        state.reconcile(projects)
        XCTAssertNil(state.selectedID)
        XCTAssertTrue(state.visibleProjects(projects).isEmpty)
    }

    func testStarFilterAndRefreshRetainOnlyVisibleProjectIDs() throws {
        let projects = try projects()
        var state = ProjectBrowserPresentation(selectedID: "second")
        state.reconcile(projects.reversed())
        XCTAssertEqual(state.selectedID, "second")
        state.starredOnly = true
        state.reconcile(projects)
        XCTAssertEqual(state.visibleProjects(projects).map(\.id), [projects[0].id])
        XCTAssertEqual(state.selectedID, projects[0].id)
        state.starredOnly = false
        state.reconcile([projects[1]])
        XCTAssertEqual(state.selectedID, "second")
        state.reconcile([])
        XCTAssertNil(state.selectedID)
    }

    func testSearchNavigationClampsAtEdgesAndCannotSubmitAnEmptyList() {
        var selection = SearchResultSelection()
        selection.move(-1, count: 7)
        XCTAssertEqual(selection.selectedIndex(count: 7), 0)
        selection.move(2, count: 7)
        XCTAssertEqual(selection.selectedIndex(count: 7), 2)
        selection.move(20, count: 7)
        XCTAssertEqual(selection.selectedIndex(count: 7), 6)
        XCTAssertNil(selection.selectedIndex(count: 0))
        XCTAssertEqual(selection.selectedIndex(count: 2), 1)
        selection.reset()
        XCTAssertEqual(selection.selectedIndex(count: 2), 0)
    }

    func testBundledWebViewWordmarksAndIconsLoadAsNativeImages() {
        for name in ["wordmark-light", "wordmark-dark"] {
            let image = WispDesign.image(name)
            XCTAssertEqual(image.size, NSSize(width: 520, height: 344))
            XCTAssertNotNil(image.tiffRepresentation)
        }
        for icon in ["search", "refresh", "database", "folder", "star", "star-filled", "chat", "doc", "sync", "clock", "copy", "more"] {
            let image = WispDesign.image("icon-\(icon)")
            XCTAssertEqual(image.size, NSSize(width: 24, height: 24))
            XCTAssertNotNil(image.tiffRepresentation)
        }
    }

    func testBothWebViewPalettesHaveNativeDecodableColors() {
        for (theme, palette) in WispDesign.palettes {
            for key in palette.keys {
                let color = NSColor(WispDesign.color(key, theme == "dark" ? .dark : .light))
                XCTAssertNotNil(color.usingColorSpace(.sRGB))
            }
        }
    }
}
