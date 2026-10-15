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
    @Test fun tabsKeepIndependentNavigationAndConversationDrafts() {
        val project = Page(PageKind.SESSIONS, "研究", "project")
        val home = WorkspaceState(stack = listOf(WorkspaceTab.CONVERSATIONS.root, project), drafts = mapOf(route to "未发送"))
        val calendar = home.switchingTab(WorkspaceTab.CALENDAR)
        assertEquals(PageKind.CALENDAR, calendar.page.kind)
        assertTrue(calendar.hasBack)
        assertTrue(calendar.showsTabs)
        val restored = calendar.switchingTab(WorkspaceTab.CONVERSATIONS)
        assertEquals(home.stack, restored.stack)
        assertEquals("未发送", restored.drafts[route])
        assertFalse(restored.copy(stack = restored.stack + Page(PageKind.CONVERSATION, "对话", "project", "session")).showsTabs)
        assertFalse(WorkspaceState().hasBack)
    }
    @Test fun journeyMonthDoesNotMoveTheCalendarTab() {
        val day = java.time.LocalDate.of(2026, 10, 14)
        val calendar = WorkspaceState(month = java.time.YearMonth.from(day), selectedDay = day, journeyMonth = java.time.YearMonth.from(day))
        val journey = calendar.shiftingJourneyMonth(-1)
        assertEquals(java.time.YearMonth.of(2026, 9), journey.journeyMonth)
        assertEquals(calendar.month, journey.month)
        assertEquals(day, journey.selectedDay)
    }
    @Test fun transcriptDetailIsADeviceSettingThatSurvivesClearingTheRemoteView() {
        assertEquals(TranscriptDetail.STANDARD, WorkspaceState().transcriptDetail)
        val note = LocalEntry("note", "notebook", "观察", "文字")
        val cleared = state.copy(transcriptDetail = TranscriptDetail.MINIMAL, entries = listOf(note), detailMenu = true,
            drafts = mapOf(route to "草稿")).cleared(demo = true)
        assertEquals(TranscriptDetail.MINIMAL, cleared.transcriptDetail)
        assertEquals(listOf(note), cleared.entries)
        assertTrue(cleared.demo)
        assertNull(cleared.snapshot)
        assertTrue(cleared.drafts.isEmpty())
        assertFalse(cleared.detailMenu)
        assertEquals(listOf(WorkspaceTab.CONVERSATIONS.root), cleared.stack)
    }
    @Test fun openDetailMenuIsTheTopmostBackLayer() {
        assertFalse(WorkspaceState().hasBack)
        assertTrue(WorkspaceState(detailMenu = true).hasBack)
    }
    @Test fun hostReasoningItemsAreLabelledAsThinking() {
        assertEquals("思考", roleLabel("reasoning"))
        assertEquals("工具", roleLabel("tool"))
    }
}
