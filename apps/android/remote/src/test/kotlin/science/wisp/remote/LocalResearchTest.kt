package science.wisp.remote

import java.time.LocalDate
import kotlin.test.*
import kotlinx.serialization.json.*

class LocalResearchTest {
    private val today = LocalDate.of(2026, 10, 14)
    private val entry = LocalEntry("1", "checklist", "复核数据", "复核数据")
    @Test fun existingUndatedRecordsSurviveAndNewDatesRoundTrip() {
        val legacy = """{"schema":1,"entries":[{"id":"1","kind":"checklist","title":"复核数据","text":"复核数据","done":false}]}"""
        assertEquals(listOf(entry), LocalEntries.decode(legacy))
        assertEquals(entry.copy(dueDate = today), LocalEntries.decode(LocalEntries.encode(listOf(entry.copy(dueDate = today)))).single())
        assertFails { LocalEntries.decode(legacy.replace("\"done\":false", "\"done\":false,\"due_date\":\"2026-02-30\"")) }
    }
    @Test fun completingOrReschedulingMovesExactlyTheAffectedSection() {
        assertEquals(ChecklistSection.UNDATED, entry.checklistSection(today))
        assertEquals(ChecklistSection.OVERDUE, entry.copy(dueDate = today.minusDays(1)).checklistSection(today))
        assertEquals(ChecklistSection.TODAY, entry.copy(dueDate = today).checklistSection(today))
        assertEquals(ChecklistSection.UPCOMING, entry.copy(dueDate = today.plusDays(1)).checklistSection(today))
        assertEquals(ChecklistSection.COMPLETED, entry.copy(done = true, dueDate = today.minusDays(1)).checklistSection(today))
    }
    @Test fun failuresAndTimeoutsAreNeverHiddenAsCompleted() {
        val runs = listOf("succeeded", "failed", "timed_out", "running", "lost", "cancelled").map { obj("status" to string(it)) }
        assertEquals(listOf(runs.first()), completedRuns(runs))
    }
}
