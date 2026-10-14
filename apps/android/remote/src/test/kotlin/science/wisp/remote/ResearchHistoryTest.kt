package science.wisp.remote

import java.time.YearMonth
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
}
