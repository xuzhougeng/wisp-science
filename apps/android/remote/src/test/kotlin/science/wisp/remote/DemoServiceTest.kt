package science.wisp.remote

import kotlin.test.*
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.*

class DemoServiceTest {
    private val assets = mutableListOf<String>()
    private fun demo() = DemoService(fixture("manifest.json").toString()) { path ->
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
}
