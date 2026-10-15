package science.wisp.remote

import kotlin.test.*
import kotlinx.serialization.json.*

class RemoteTranscriptTest {
    private fun item(role: String, text: String = "", tool: String = "", ok: Boolean? = null): JsonElement =
        obj("role" to string(role), "text" to string(text), "tool_name" to string(tool), "ok" to (ok?.let { JsonPrimitive(it) } ?: JsonNull))

    private fun expected(row: JsonElement): TranscriptRow {
        fun int(key: String) = row.field(key).jsonPrimitive.int
        val indices = row.field("indices").rows().map { it.jsonPrimitive.int }
        return when (row.field("kind").text()) {
            "item" -> TranscriptRow.Item(int("index"))
            "process" -> TranscriptRow.Process(indices, int("tools"), int("files"), int("failures"))
            "failures" -> TranscriptRow.Failures(indices, int("count"))
            else -> fail("Unknown row kind: $row")
        }
    }

    @Test fun sharedFixturePinsEveryCaseAtEveryLevel() {
        val contract = fixture("native-conversations/v1/transcript-detail.json")
        assertEquals(contract.field("levels").rows().map { it.text() }, TranscriptDetail.entries.map { it.id })
        assertEquals(contract.field("default_level").text(), TranscriptDetail.DEFAULT.id)
        TranscriptDetail.entries.forEach { assertEquals(contract.field("labels").field(it.id).text(), it.label) }
        val cases = contract.field("cases").rows()
        assertTrue(cases.isNotEmpty())
        cases.forEach { case ->
            val items = case.field("items").rows()
            TranscriptDetail.entries.forEach { level ->
                assertEquals(case.field("expect").field(level.id).rows().map(::expected),
                    RemoteTranscript.rows(items, case.field("running").flag(), level), "${case.field("name").text()} / ${level.id}")
            }
        }
    }

    @Test fun toolStepsAndChangedPathsCollapseWithoutHidingTheAnswer() {
        val items = listOf(
            item("user", "绘制表达热图"), item("reasoning", "检查数据"), item("assistant", "准备绘图"),
            item("tool", "done", tool = "shell", ok = true),
            item("file_changed", "figures/heatmap.png"), item("file_changed", "figures/heatmap.pdf"),
            item("acp_tool", "missing", tool = "read", ok = false),
            item("file_changed", "figures/heatmap.png"),
            item("assistant", "# 分析完成\n\n[查看报告](results/report.md)"), item("usage", "{}"),
        )
        val groups = RemoteTranscript.groups(items)
        assertEquals(listOf(false, true, false, false), groups.map { it.isProcess })
        assertEquals(2, groups[1].toolCount)
        assertEquals(2, groups[1].changedFileCount, "Count distinct paths, not write notifications")
        assertEquals(1, groups[1].failureCount)
        assertEquals(listOf(items[8]), groups[2].items)
        assertEquals(items, groups.flatMap { it.items }, "Grouping must preserve every item and its order")
    }

    @Test fun questionsPlansWarningsAndUnknownRolesRemainVisibleAndSeparateTurns() {
        listOf("question", "plan", "system", "user", "queued_user", "review", "future-role").forEach { role ->
            val boundary = item(role, "需要你查看")
            val groups = RemoteTranscript.groups(listOf(item("tool", tool = "read"), boundary, item("tool", tool = "edit")))
            assertEquals(listOf(true, false, true), groups.map { it.isProcess }, role)
            assertEquals(listOf(boundary), groups[1].items, role)
        }
    }

    @Test fun resultToolsAndAssistantResourcesAreNotFoldedAway() {
        listOf("attempt_completion", "monitor_run", "wisp_monitor_run", "generate_image", "generate_video").forEach { tool ->
            val items = listOf(item("reasoning"), item("tool", tool = tool, ok = true), item("file_changed", "figure.png"))
            assertEquals(listOf(true, false, true), RemoteTranscript.groups(items).map { it.isProcess }, tool)
        }
        val resource = obj("role" to string("assistant"), "text" to string("![图](fig.png)"),
            "resources" to JsonArray(listOf(obj("id" to string("image-a")))))
        assertEquals(listOf(false, true), RemoteTranscript.groups(listOf(resource, item("tool", tool = "read"))).map { it.isProcess })
    }

    @Test fun currentCommentaryRemainsReadableWhilePastWorkStaysGrouped() {
        val items = listOf(item("user", "上一轮"), item("assistant", "旧进度"), item("tool", tool = "read"),
            item("assistant", "旧回答"), item("user", "这一轮"), item("assistant", "正在绘图"),
            item("tool", tool = "shell"), item("file_changed", "figure.png"))
        val live = RemoteTranscript.groups(items, running = true)
        assertEquals(listOf(false, true, false, false, false, true), live.map { it.isProcess })
        assertEquals(listOf(items[5]), live[4].items)
        assertEquals(items.subList(5, items.size), RemoteTranscript.groups(items).last().items)
        assertTrue(RemoteTranscript.groups(emptyList(), running = true).isEmpty())
        assertTrue(RemoteTranscript.rows(emptyList(), running = true, TranscriptDetail.MINIMAL).isEmpty())
    }

    @Test fun groupIdentitySurvivesAppendedStepsAndEarlierPages() {
        val live = listOf(item("user", "当前问题"), item("tool", tool = "shell"))
        val original = RemoteTranscript.groups(live)
        val extended = RemoteTranscript.groups(live + listOf(item("tool", tool = "read"), item("file_changed", "data.csv")))
        assertEquals(original.last().id, extended.last().id)
        val older = listOf(item("user", "旧问题"), item("tool", tool = "read"), item("assistant", "旧回答"))
        val paged = RemoteTranscript.groups(older + live, indexOffset = -older.size)
        assertEquals(original, paged.takeLast(original.size))
        assertEquals(paged.size, paged.map { it.id }.toSet().size)
    }

    @Test fun emptyAssistantPlaceholdersDoNotEndCommentaryLookAhead() {
        val items = listOf(item("user", "继续"), item("assistant", "先读取数据"), item("assistant"), item("tool", tool = "read"))
        assertEquals(listOf(TranscriptRow.Item(0), TranscriptRow.Process(listOf(1, 2, 3), 1, 0, 0)),
            RemoteTranscript.rows(items, running = false, TranscriptDetail.STANDARD))
    }

    @Test fun failedLostAndRejectedToolsCountButOtherRolesDoNot() {
        val failed = obj("role" to string("tool"), "tool_name" to string("shell"), "text" to string(""), "status" to string("failed"))
        val lost = obj("role" to string("acp_tool"), "tool_name" to string("ssh"), "text" to string(""), "status" to string("lost"))
        val items = listOf(item("user", "运行"), failed, lost, item("tool", tool = "read", ok = false),
            item("tool", tool = "edit", ok = true), item("reasoning", ok = false))
        assertEquals(listOf(TranscriptRow.Item(0), TranscriptRow.Failures(listOf(1, 2, 3, 4, 5), 3)),
            RemoteTranscript.rows(items, running = false, TranscriptDetail.MINIMAL))
    }

    @Test fun processSummaryOmitsEmptyCounts() {
        assertEquals("执行过程 · 2 步 · 2 个文件", TranscriptRow.Process(listOf(1), 2, 2, 0).summary)
        assertEquals("执行过程 · 1 步", TranscriptRow.Process(listOf(1), 1, 0, 1).summary)
        assertEquals("执行过程", TranscriptRow.Process(listOf(1), 0, 0, 0).summary)
    }

    @Test fun storedLevelFallsBackToStandard() {
        TranscriptDetail.entries.forEach { assertEquals(it, TranscriptDetail.from(it.id)) }
        assertEquals(TranscriptDetail.STANDARD, TranscriptDetail.from(null))
        assertEquals(TranscriptDetail.STANDARD, TranscriptDetail.from("verbose"))
    }
}
