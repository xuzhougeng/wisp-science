package science.wisp.remote

import java.time.LocalDate
import java.time.YearMonth
import java.time.ZoneId
import kotlinx.serialization.json.*

object ResearchHistory {
    fun monthArgs(month: YearMonth, zone: ZoneId): JsonObject = obj(
        "from" to JsonPrimitive(month.atDay(1).atStartOfDay(zone).toEpochSecond()),
        "until" to JsonPrimitive(month.plusMonths(1).atDay(1).atStartOfDay(zone).toEpochSecond()),
    )

    fun dayArgs(day: LocalDate, zone: ZoneId): JsonObject = obj(
        "from" to JsonPrimitive(day.atStartOfDay(zone).toEpochSecond()),
        "until" to JsonPrimitive(day.plusDays(1).atStartOfDay(zone).toEpochSecond()),
    )

    fun monthCells(month: YearMonth): List<LocalDate?> {
        val offset = month.atDay(1).dayOfWeek.value - 1
        val count = ((offset + month.lengthOfMonth() + 6) / 7) * 7
        return List(count) { index -> (index - offset + 1).takeIf { it in 1..month.lengthOfMonth() }?.let(month::atDay) }
    }

    fun parse(value: JsonElement): JsonObject {
        requireRemote(value.field("entries") is JsonArray &&
            value.field("truncated").let { it == JsonPrimitive(true) || it == JsonPrimitive(false) })
        val entries = value.field("entries").rows()
        requireRemote(entries.all {
            it.field("id").text().isNotBlank() && it.field("kind").text().isNotBlank() &&
                (it.field("occurred_at") as? JsonPrimitive)?.longOrNull != null
        })
        return obj("entries" to JsonArray(entries.distinctBy { it.field("id").text() }
            .sortedByDescending { it.field("occurred_at").jsonPrimitive.long }), "truncated" to value.field("truncated"))
    }
}

/** Projection of native-calendar/v1. Errors and privacy-filtered omissions are not empty days. */
data class CalendarHistory(val entries: List<JsonElement> = emptyList(), val unavailable: Set<String> = emptySet(), val truncated: Boolean = false) {
    companion object {
        fun parse(value: JsonElement, requested: Set<String>): CalendarHistory {
            requireRemote(value is JsonArray)
            val rows = value.rows()
            val ids = rows.map { it.field("project_id").text() }
            requireRemote(ids.all { it in requested } && ids.distinct().size == ids.size)
            val missing = (requested - ids.toSet()).toMutableSet()
            var truncated = false
            val entries = rows.flatMap { row ->
                val id = row.field("project_id").text()
                if (row.field("error") != JsonNull) {
                    requireRemote(row.field("error").text().isNotBlank())
                    missing += id
                    emptyList()
                } else {
                    val history = ResearchHistory.parse(row.field("history"))
                    truncated = truncated || history.field("truncated").flag()
                    history.field("entries").rows().map { JsonObject(it.jsonObject + ("project_id" to string(id))) }
                }
            }
            return CalendarHistory(entries.sortedByDescending { it.field("occurred_at").jsonPrimitive.long }, missing, truncated)
        }

        suspend fun load(projects: List<String>, range: JsonObject, read: suspend (JsonObject) -> JsonElement): CalendarHistory {
            val batches = projects.distinct().chunked(50).map { ids ->
                parse(read(JsonObject(range + ("project_ids" to JsonArray(ids.map(::string))))), ids.toSet())
            }
            return CalendarHistory(batches.flatMap { it.entries }.sortedByDescending { it.field("occurred_at").jsonPrimitive.long },
                batches.flatMap { it.unavailable }.toSet(), batches.any { it.truncated })
        }
    }
}
