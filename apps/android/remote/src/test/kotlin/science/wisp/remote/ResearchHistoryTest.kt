package science.wisp.remote

import java.time.LocalDate
import java.time.YearMonth
import kotlinx.coroutines.test.runTest
import java.time.ZoneId
import kotlin.test.*
import kotlinx.serialization.json.*

class ResearchHistoryTest {
    @Test fun monthRangesUseLocalCalendarAcrossDstLeapYearsAndYearBoundary() {
        val zone = ZoneId.of("America/Los_Angeles")
        val spring = ResearchHistory.monthArgs(YearMonth.of(2026, 3), zone)
        assertEquals(31 * 86400L - 3600, spring.field("until").jsonPrimitive.long - spring.field("from").jsonPrimitive.long)
        val leap = ResearchHistory.monthArgs(YearMonth.of(2024, 2), zone)
        assertEquals(29 * 86400L, leap.field("until").jsonPrimitive.long - leap.field("from").jsonPrimitive.long)
        val december = ResearchHistory.monthArgs(YearMonth.of(2026, 12), zone)
        val january = ResearchHistory.monthArgs(YearMonth.of(2027, 1), zone)
        assertEquals(december.field("until"), january.field("from"))
    }
    @Test fun sharedJourneyFixturePreservesTruncationAndRejectsMalformedHistory() {
        val value = fixture("native-journey/v1/range.json").field("result")
        val entry = value.field("entries").rows().single()
        val parsed = ResearchHistory.parse(obj("entries" to JsonArray(listOf(entry, entry)), "truncated" to JsonPrimitive(true)))
        assertEquals(1, parsed.field("entries").rows().size)
        assertTrue(parsed.field("truncated").flag())
        assertFailsWith<RemoteFailure> { ResearchHistory.parse(obj()) }
        assertFailsWith<RemoteFailure> { ResearchHistory.parse(obj("entries" to JsonArray(listOf(obj())), "truncated" to JsonPrimitive(false))) }
    }
    @Test fun selectedDayUsesLocalMidnightAndMondayFirstGrid() {
        val zone = ZoneId.of("America/Los_Angeles")
        val spring = ResearchHistory.dayArgs(LocalDate.of(2026, 3, 8), zone)
        assertEquals(23 * 3600L, spring.field("until").jsonPrimitive.long - spring.field("from").jsonPrimitive.long)
        val fall = ResearchHistory.dayArgs(LocalDate.of(2026, 11, 1), zone)
        assertEquals(25 * 3600L, fall.field("until").jsonPrimitive.long - fall.field("from").jsonPrimitive.long)
        val cells = ResearchHistory.monthCells(YearMonth.of(2026, 10))
        assertEquals(35, cells.size)
        assertEquals(listOf(null, null, null, LocalDate.of(2026, 10, 1)), cells.take(4))
        assertEquals(31, cells.filterNotNull().size)
        assertEquals(42, ResearchHistory.monthCells(YearMonth.of(2026, 3)).size)
    }
    @Test fun calendarFixtureAndMissingProjectsCannotMasqueradeAsEmptyDays() {
        val value = fixture("native-calendar/v1/month.json").field("result")
        val history = CalendarHistory.parse(value, setOf("research-1", "hidden"))
        assertEquals(setOf("hidden"), history.unavailable)
        assertEquals("research-1", history.entries.single().field("project_id").text())
        assertFailsWith<RemoteFailure> { CalendarHistory.parse(value, setOf("other")) }
        assertFailsWith<RemoteFailure> { CalendarHistory.parse(JsonArray(value.rows() + value.rows()), setOf("research-1")) }
        val failure = obj("project_id" to string("failed"), "error" to string("unavailable"))
        assertEquals(setOf("failed"), CalendarHistory.parse(JsonArray(listOf(failure)), setOf("failed")).unavailable)
        assertFailsWith<RemoteFailure> { CalendarHistory.parse(JsonArray(listOf(obj("project_id" to string("broken")))), setOf("broken")) }
    }
    @Test fun largeCalendarsBatchEveryProjectAndPreserveTruncation() = runTest {
        val calls = mutableListOf<List<String>>()
        val ids = (1..123).map { "p$it" }
        val history = CalendarHistory.load(ids + ids.first(), ResearchHistory.monthArgs(YearMonth.of(2026, 10), ZoneId.of("UTC"))) { args ->
            val batch = args.field("project_ids").rows().map { it.text() }
            calls += batch
            JsonArray(batch.map { obj("project_id" to string(it), "error" to JsonNull,
                "history" to obj("entries" to JsonArray(emptyList()), "truncated" to JsonPrimitive(it == "p123"))) })
        }
        assertEquals(listOf(50, 50, 23), calls.map { it.size })
        assertEquals(ids, calls.flatten())
        assertTrue(history.truncated)
        assertTrue(history.unavailable.isEmpty())
    }
}
