package science.wisp.android

import kotlin.test.*
import kotlinx.serialization.json.*
import science.wisp.remote.*

class WorkspaceStateTest {
    private val route = ConversationRoute("project", "session")
    private val snapshot = RemoteSnapshot(obj(
        "schema" to string("wisp.native-conversations.v1"), "project_id" to string(route.project),
        "session_id" to string(route.session), "epoch" to string("host"), "sequence" to JsonPrimitive(1),
        "running" to JsonPrimitive(false), "stopping" to JsonPrimitive(false), "read_only" to JsonPrimitive(false),
        "items" to JsonArray(emptyList()), "approvals" to JsonArray(emptyList()),
    ), route)
    private val state = WorkspaceState(
        stack = listOf(Page(PageKind.CONVERSATION, "Chat", route.project, route.session)),
        snapshot = snapshot, snapshotSession = "connection",
        status = ConnectionStatus(ConnectionPhase.ONLINE, "connection", commands = setOf("send")),
    )
    @Test fun busyAndUncertainOutcomesCannotBeSentAgain() {
        assertTrue(state.can("send"))
        assertFalse(state.copy(busy = true).can("send"))
        assertFalse(state.copy(uncertain = setOf(route)).can("send"))
    }
    @Test fun snapshotCannotAuthorizeAnotherConversation() {
        assertFalse(state.copy(stack = listOf(Page(PageKind.CONVERSATION, "Other", route.project, "other"))).can("send"))
    }
    @Test fun disconnectedAndReconnectedSnapshotsAreReadOnlyUntilRefreshed() {
        assertFalse(state.copy(status = state.status.copy(phase = ConnectionPhase.OFFLINE)).can("send"))
        assertFalse(state.copy(status = state.status.copy(session = "reconnected")).can("send"))
        assertFalse(state.copy(snapshotSession = "").can("send"))
    }
}
