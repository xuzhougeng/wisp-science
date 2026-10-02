import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

final class NativeTranscriptArtifactTests: XCTestCase {
    private func item(_ text: String, role: String = "assistant") throws -> ConversationItem {
        try JSONDecoder().decode(ConversationItem.self, from: JSONSerialization.data(withJSONObject: ["role": role, "text": text]))
    }
    func testRichReplyIncludesTableAndCompleteFormulaWithoutPersistentFiles() throws {
        let source = """
        | Sample | Reads |
        | :--- | ---: |
        | A | 12 |
        | B | 24 |

        $$ y = \\alpha + \\beta x $$
        """
        let values = NativeTranscriptArtifact.collect([try item(source)])
        XCTAssertEqual(values.map(\.kind), ["table", "latex"])
        XCTAssertEqual(values.map(\.title), ["表格 1", "公式 1"])
        XCTAssertEqual(values[1].source, "y = \\alpha + \\beta x")
        XCTAssertTrue(values[0].source.contains("| B | 24 |"))
        XCTAssertEqual(values[0].tableDimensions?.rows, 2)
        XCTAssertEqual(values[0].tableDimensions?.columns, 2)
        let groups = NativeArtifactGroup.collect(registered: [], messages: values, query: "")
        XCTAssertEqual(groups.map(\.id), ["latex", "table"])
        XCTAssertEqual(groups.map(\.count), [1, 1])
        XCTAssertEqual(NativeArtifactGroup.collect(registered: [], messages: values, query: "24").map(\.id), ["table"])
        XCTAssertTrue(NativeArtifactGroup.collect(registered: [], messages: values, query: "no such sample").isEmpty)
        XCTAssertEqual(values, NativeTranscriptArtifact.collect([try item(source)]))
        XCTAssertTrue(NativeTranscriptArtifact.collect([]).isEmpty)
    }
    func testInspectorClampsWidePreferenceToKeepComposerAvailable() {
        XCTAssertEqual(ProjectWorkspace.panelWidth(preferred: 600, available: 900, sidebar: true), 291)
        XCTAssertEqual(ProjectWorkspace.panelWidth(preferred: 380, available: 1100, sidebar: true), 380)
        XCTAssertEqual(ProjectWorkspace.panelWidth(preferred: 600, available: 900, sidebar: false), 540)
        XCTAssertEqual(ProjectWorkspace.panelWidth(preferred: 380, available: 680, sidebar: false), 320)
    }
    func testCodeUserToolAndIncompleteStreamsDoNotProduceArtifacts() throws {
        let table = "| A | B |\n| --- | --- |\n| 1 | 2 |"
        let fenced = "````python\n```\n" + table + "\n$$ x $$\n````\n~~~\n$$ y $$\n~~~"
        XCTAssertTrue(NativeTranscriptArtifact.collect([try item(fenced), try item(table, role: "user"), try item(table, role: "tool")]).isEmpty)
        XCTAssertTrue(NativeTranscriptArtifact.collect([try item("$$\nx + y\n")]).isEmpty)
        XCTAssertTrue(NativeTranscriptArtifact.collect([try item("    $$ x $$\n    | A | B |\n    | --- | --- |")]).isEmpty)
        XCTAssertTrue(NativeTranscriptArtifact.collect([try item("a | b\n- | -")]).isEmpty)
        XCTAssertTrue(NativeTranscriptArtifact.collect([try item("a | b\n--- | --- | ---")]).isEmpty)
    }
    func testMultilineFormulaPreservesSourceAndNumberingAcrossMessages() throws {
        let values = NativeTranscriptArtifact.collect([try item("$$\nx + y\n= z\n$$"), try item("$$ a $$\n\n$$ b $$")])
        XCTAssertEqual(values.map(\.title), ["公式 1", "公式 2", "公式 3"])
        XCTAssertEqual(values[0].source, "x + y\n= z")
        XCTAssertEqual(Set(values.map(\.id)).count, 3)
    }
}
