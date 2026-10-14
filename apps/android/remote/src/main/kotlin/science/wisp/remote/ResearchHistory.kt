package science.wisp.remote

import java.time.YearMonth
import java.time.ZoneId
import kotlinx.serialization.json.*

object ResearchHistory {
    fun monthArgs(month: YearMonth, zone: ZoneId): JsonObject = obj(
        "from" to JsonPrimitive(month.atDay(1).atStartOfDay(zone).toEpochSecond()),
        "until" to JsonPrimitive(month.plusMonths(1).atDay(1).atStartOfDay(zone).toEpochSecond()),
    )

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
