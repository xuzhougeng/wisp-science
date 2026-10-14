package science.wisp.remote

import kotlin.test.*
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.*

class DemoServiceTest {
    private val assets = mutableListOf<String>()
    private fun demo() = DemoService(fixture("manifest.json").toString(), readImageAsset = { path ->
        requireNotNull(javaClass.classLoader!!.getResourceAsStream(path)).use { it.readBytes() }
    }) { path ->
        assets += path
        requireNotNull(javaClass.classLoader!!.getResourceAsStream(path)).bufferedReader().use { it.readText() }
    }
    private val route = ConversationRoute("demo-rnaseq", "demo-qc")

    @Test fun bundledDemoSupportsMessagesFilesAndResetWithoutATransport() = runTest {
        val demo = demo()
        assertEquals("demo-rnaseq", demo.call("list_projects").rows().single().field("id").text())
        val before = RemoteSnapshot(demo.call("native_conversation_snapshot", route.project, route.args), route)
        demo.call("native_conversation_send", route.project, JsonObject(route.args + ("message" to string("测试"))))
        val after = RemoteSnapshot(demo.call("native_conversation_snapshot", route.project, route.args), route)
        assertEquals(before.items.size + 2, after.items.size)
        assertTrue(after.items.last().field("text").text().contains("本地预设回复"))
        val preview = demo.call("native_conversation_panel_file_read", route.project, JsonObject(route.args + ("path" to string("results/qc_report.md"))))
        assertTrue(preview.field("content").field("text").text().isNotBlank())
        assertEquals(listOf("files/results/qc_report.md"), assets)
        assertEquals(before.items.size, RemoteSnapshot(demo().call("native_conversation_snapshot", route.project, route.args), route).items.size)
    }
    @Test fun demoRejectsOtherProjectsSessionsAndTraversal() = runTest {
        val demo = demo()
        assertFailsWith<RemoteFailure> { demo.call("remote_sessions", "other") }
        assertFailsWith<RemoteFailure> { demo.call("native_conversation_snapshot", route.project, obj("session_id" to string("other"))) }
        listOf("../manifest.json", "/etc/passwd", "results/../../manifest.json", "results\\qc_report.md").forEach { path ->
            assertFailsWith<RemoteFailure> { demo.call("native_conversation_panel_file_read", route.project, JsonObject(route.args + ("path" to string(path)))) }
        }
        assertTrue(assets.isEmpty())
    }
    @Test fun approvalIsOnceOnlyAndQuestionsDoNotExposeInternalNotes() = runTest {
        val demo = demo()
        val args = obj("session_id" to string("demo-workflow"), "approval_id" to string("demo-approval"),
            "scope" to string("once"), "approved" to JsonPrimitive(false))
        demo.call("native_conversation_approve", route.project, args)
        assertTrue(demo.call("native_conversation_snapshot", route.project, args).field("approvals").rows().isEmpty())
        assertFailsWith<RemoteFailure> { demo.call("native_conversation_approve", route.project, args) }
        val questionItem = demo.call("native_conversation_snapshot", route.project,
            obj("session_id" to string("demo-options"))).field("items").rows().last()
        val question = assertNotNull(Question.parse(questionItem))
        assertEquals(3, question.options.size)
        assertFalse(question.text.contains("End your turn"))
    }
    @Test fun demoCalendarAndInboxUseTheSameProjectsAndApprovalState() = runTest {
        val demo = demo()
        val project = demo.call("list_projects").rows().single()
        assertEquals(3, project.field("session_count").jsonPrimitive.int)
        val inbox = demo.call("native_conversation_inbox", route.project).rows()
        assertTrue(inbox.any { it.field("id").text() == "demo-workflow" })
        val range = ResearchHistory.dayArgs(java.time.LocalDate.now(), java.time.ZoneId.systemDefault())
        val history = CalendarHistory.load(listOf(route.project), range) { demo.call("native_research_calendar", args = it) }
        assertEquals("demo-qc", history.entries.single().field("frame_id").text())
        val emptyDay = ResearchHistory.dayArgs(java.time.LocalDate.now().plusDays(10), java.time.ZoneId.systemDefault())
        assertTrue(demo.call("native_research_journey", route.project, emptyDay).field("entries").rows().isEmpty())
        demo.call("native_conversation_approve", route.project, obj("session_id" to string("demo-workflow"), "approval_id" to string("demo-approval"), "approved" to JsonPrimitive(false), "scope" to string("once")))
        // Like the iOS demo, an unseen reply still needs the user until the conversation is marked seen.
        assertTrue(demo.call("native_conversation_inbox", route.project).rows().any { it.field("id").text() == "demo-workflow" })
        demo.call("native_conversation_seen", route.project, obj("session_id" to string("demo-workflow")))
        assertFalse(demo.call("native_conversation_inbox", route.project).rows().any { it.field("id").text() == "demo-workflow" })
        assertEquals("idle", demo.call("remote_sessions", route.project).rows().single { it.field("id").text() == "demo-workflow" }.field("status").text())
    }
    @Test fun seenClearsOnlyUnreadStateAndRejectsUnknownSessions() = runTest {
        val demo = demo()
        assertEquals(setOf("demo-qc", "demo-workflow", "demo-options"), demo.call("native_conversation_inbox", route.project).rows().map { it.field("id").text() }.toSet())
        listOf("demo-qc", "demo-workflow", "demo-options").forEach { demo.call("native_conversation_seen", route.project, obj("session_id" to string(it))) }
        // The pending approval and the unanswered question keep their conversations in the inbox.
        assertEquals(setOf("demo-workflow", "demo-options"), demo.call("native_conversation_inbox", route.project).rows().map { it.field("id").text() }.toSet())
        assertFailsWith<RemoteFailure> { demo.call("native_conversation_seen", route.project, obj("session_id" to string("other"))) }
    }
    @Test fun bundledImageUsesTheBoundedImageCommandAndRejectsRemotePaths() = runTest {
        val demo = demo()
        val value = demo.call("native_conversation_image", route.project, JsonObject(route.args + ("path" to string("results/qc_summary.png"))))
        val bytes = ImagePreview.bytes(value)
        assertContentEquals(byteArrayOf(0x89.toByte(), 0x50, 0x4e, 0x47), bytes.take(4).toByteArray())
        assertFailsWith<RemoteFailure> { demo.call("native_conversation_image", route.project, JsonObject(route.args + ("path" to string("https://example.org/a.png")))) }
        assertFailsWith<RemoteFailure> { demo.call("native_conversation_image", route.project, JsonObject(route.args + mapOf("path" to string("results/qc_summary.png"), "resource_id" to string("unavailable")))) }
    }
}
