package science.wisp.remote

import kotlin.test.*
import kotlinx.serialization.json.*
import org.commonmark.node.*
import org.commonmark.ext.gfm.tables.*
import org.commonmark.ext.gfm.strikethrough.Strikethrough
import org.commonmark.ext.task.list.items.TaskListItemMarker

class MarkdownTest {
    private fun nodes(node: Node): List<Node> = listOf(node) + node.children().flatMap(::nodes)

    @Test fun commonMarkParsesStructureInsteadOfLeakingMarkup() {
        val text = """
            # 结果

            **重点**、*强调*、~~旧结论~~ 与 `x <- 1`。[报告](results/report.md)

            > 引用证据

            3. 第三步
               - 嵌套事项

            - [x] 复核完成

            ```r
            x <- 1
            print(x)
            ```
        """.trimIndent()
        val nodes = nodes(Markdown.parse(text))
        assertEquals(1, nodes.filterIsInstance<Heading>().single().level)
        assertEquals(1, nodes.filterIsInstance<StrongEmphasis>().size)
        assertEquals(1, nodes.filterIsInstance<Emphasis>().size)
        assertEquals(1, nodes.filterIsInstance<Strikethrough>().size)
        assertEquals("x <- 1", nodes.filterIsInstance<Code>().single().literal)
        assertEquals("r", nodes.filterIsInstance<FencedCodeBlock>().single().info)
        assertEquals(3, nodes.filterIsInstance<OrderedList>().single().markerStartNumber)
        assertTrue(nodes.filterIsInstance<TaskListItemMarker>().single().isChecked)
        assertEquals("results/report.md", nodes.filterIsInstance<Link>().single().destination)
        assertTrue(nodes.any { it is BlockQuote })
    }

    @Test fun gfmTablePreservesEscapedPipesEmphasisAndColumnAlignment() {
        val document = Markdown.parse("| 样本 | 结果 |\n| :--- | ---: |\n| A\\|B | **96.2%** |")
        val cells = nodes(document).filterIsInstance<TableCell>()
        assertEquals(4, cells.size)
        assertTrue(cells.take(2).all { it.isHeader })
        assertEquals(TableCell.Alignment.RIGHT, cells.last().alignment)
        assertEquals("A|B", nodes(cells[2]).filterIsInstance<Text>().single().literal)
        assertTrue(nodes(cells.last()).any { it is StrongEmphasis })
    }

    @Test fun bundledReportParsesTablesImagesAndProjectLinks() {
        val report = fixture("manifest.json").field("sessions").rows().first().field("items").rows().last().field("text").text()
        val nodes = nodes(Markdown.parse(report))
        assertEquals(7, nodes.filterIsInstance<TableRow>().size)
        assertEquals("results/qc_summary.png", nodes.filterIsInstance<Image>().single().destination)
        assertEquals(listOf("results/qc_report.md", "data/samples.csv"), nodes.filterIsInstance<Link>().map { it.destination })
    }

    @Test fun linksResolveRelativeToDocumentWithoutAllowingActiveSchemes() {
        assertEquals(Markdown.Target.ProjectFile("data/sample table.csv"), Markdown.target("../data/sample%20table.csv#L2", "results"))
        assertEquals(Markdown.Target.ProjectFile("results/report.md"), Markdown.target("./report.md", "results"))
        assertEquals(Markdown.Target.ProjectFile("/work/results/a.png"), Markdown.target("/work/results/a.png"))
        assertEquals(Markdown.Target.Website("https://example.org/paper?q=1"), Markdown.target("https://example.org/paper?q=1"))
        listOf("javascript:alert(1)", "data:text/html,test", "file:///etc/passwd", "intent://run", "//example.org/a", "%2F%2Fexample.org/a",
            "https://user:password@example.org", "../outside", "bad%00file", "C:\\file", "#heading").forEach { assertNull(Markdown.target(it), it) }
    }

    @Test fun boundImagesNeverFallBackToMutablePathsOrExternalDownloads() {
        val item = fixture("native-conversations/v1/snapshot-images-queue.json").field("items").rows().last()
        val resources = item.field("resources").rows()
        assertEquals(obj("resource_id" to string("resource-a")), Markdown.imageSource("figures/qc.png", resources))
        val unavailable = JsonObject(resources.single().jsonObject + ("status" to string("unavailable")))
        assertEquals(obj("resource_id" to string("resource-a")), Markdown.imageSource("figures/qc.png", listOf(unavailable)))
        assertNull(Markdown.imageSource("https://example.org/tracking.png", emptyList()))
        assertEquals(obj("path" to string("results/chart.png")), Markdown.imageSource("chart.png", emptyList(), "results"))
        assertFailsWith<RemoteFailure> { ImagePreview.bytes(obj("base64" to string("not valid!"), "mime" to string("image/png"))) }
        assertFailsWith<RemoteFailure> { ImagePreview.bytes(obj("base64" to string("YQ=="), "mime" to string("text/html"))) }
    }
}
