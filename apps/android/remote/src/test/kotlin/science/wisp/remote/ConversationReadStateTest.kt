package science.wisp.remote

import kotlin.test.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.*

class ConversationReadStateTest {
    private val route = ConversationRoute("project", "session")
    private fun snapshot(sequence: Int, running: Boolean = false) = RemoteSnapshot(obj(
        "schema" to string("wisp.native-conversations.v1"), "project_id" to string(route.project),
        "session_id" to string(route.session), "epoch" to string("host"), "sequence" to JsonPrimitive(sequence),
        "running" to JsonPrimitive(running), "stopping" to JsonPrimitive(false), "read_only" to JsonPrimitive(false),
        "items" to JsonArray(emptyList()), "approvals" to JsonArray(emptyList()),
    ), route)

    private class RecordingService(commands: Set<String> = setOf("native_conversation_seen")) : RemoteService {
        val calls = mutableListOf<Triple<String, String?, JsonObject>>()
        var failure: Exception? = null
        override val status = MutableStateFlow(ConnectionStatus(ConnectionPhase.ONLINE, "socket", commands = commands))
        override suspend fun call(command: String, project: String?, args: JsonObject): JsonElement {
            calls += Triple(command, project, args)
            failure?.let { throw it }
            return JsonNull
        }
        override fun disconnect() = Unit
    }

    @Test fun inboxIsReadOnceBecauseTheHostAlreadySpansProjects() = runTest {
        val calls = mutableListOf<Pair<String, String?>>()
        val rows = listOf(obj("id" to string("a"), "project_id" to string("p2")), obj("id" to string("b"), "project_id" to string("p3")))
        val inbox = loadInbox { command, project ->
            calls += command to project
            if (command == "list_projects") JsonArray(listOf("p1", "p2", "p3").map { obj("id" to string(it)) }) else JsonArray(rows)
        }
        assertEquals(rows, inbox)
        assertEquals(listOf("list_projects" to null, "native_conversation_inbox" to "p1"), calls)
    }
    @Test fun inboxWithoutProjectsOrWithMalformedRowsDoesNotGuess() = runTest {
        val calls = mutableListOf<String>()
        assertTrue(loadInbox { command, _ -> calls += command; JsonArray(emptyList()) }.isEmpty())
        assertEquals(listOf("list_projects"), calls)
        assertFailsWith<RemoteFailure> { loadInbox { _, _ -> JsonNull } }
        assertFailsWith<RemoteFailure> { loadInbox { command, _ ->
            if (command == "list_projects") JsonArray(listOf(obj("id" to string("p1")))) else obj()
        } }
    }
    @Test fun settledSnapshotIsMarkedSeenOncePerSequence() = runTest {
        val service = RecordingService()
        val marker = SeenMarker()
        marker.mark(service, snapshot(1, running = true))
        assertTrue(service.calls.isEmpty())
        marker.mark(service, snapshot(2))
        marker.mark(service, snapshot(2))
        assertEquals(listOf(Triple("native_conversation_seen", route.project, route.args)), service.calls)
        marker.mark(service, snapshot(3))
        assertEquals(2, service.calls.size)
    }
    @Test fun unsupportedOrFailedSeenIsRetriedLaterWithoutBlockingTheSnapshot() = runTest {
        val marker = SeenMarker()
        val old = RecordingService(commands = emptySet())
        marker.mark(old, snapshot(1))
        assertTrue(old.calls.isEmpty())
        val service = RecordingService().apply { failure = RemoteFailure(RemoteFailure.Reason.UNKNOWN) }
        assertFailsWith<RemoteFailure> { marker.mark(service, snapshot(1)) }
        service.failure = null
        marker.mark(service, snapshot(1))
        assertEquals(2, service.calls.size)
    }
}
