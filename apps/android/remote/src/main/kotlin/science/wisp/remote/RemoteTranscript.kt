package science.wisp.remote

import kotlinx.serialization.json.*

/**
 * Presentation-only grouping of existing native conversation items, ported from the iOS
 * RemoteTranscript. It does not discard messages or change the DTO / remote command contract.
 */
object RemoteTranscript {
    data class Group(val id: Int, val items: List<JsonElement>, val isProcess: Boolean) {
        val toolCount: Int get() = items.count { isTool(it) }
        val changedFileCount: Int get() = items.filter { it.field("role").text() == "file_changed" }.map { it.field("text").text() }.toSet().size
        val failureCount: Int get() = items.count { isFailure(it) }
    }

    // These tool rows contain deliverables or run status rather than internal
    // steps. Keep them outside the group, matching the remote web client.
    private val visibleTools = setOf("attempt_completion", "monitor_run", "wisp_monitor_run", "generate_image", "generate_video")
    fun isTool(item: JsonElement): Boolean = item.field("role").text() in setOf("tool", "acp_tool")
    fun isFailure(item: JsonElement): Boolean = isTool(item) &&
        (item.field("ok") == JsonPrimitive(false) || item.field("status").text() in setOf("failed", "timed_out", "lost"))
    private fun isStep(item: JsonElement): Boolean = isTool(item) && item.field("tool_name").text() !in visibleTools

    /**
     * `indexOffset = -older.count` keeps existing live-row identities stable when an earlier
     * history page is prepended. Reset the view at epoch changes, since positions then refer
     * to a different snapshot window.
     */
    fun groups(items: List<JsonElement>, running: Boolean = false, indexOffset: Int = 0): List<Group> {
        val liveFrom = if (running) items.indexOfLast { it.field("role").text() in setOf("user", "queued_user") } else items.size
        // Look ahead once, ignoring reasoning, file notifications and empty
        // assistant placeholders, to distinguish commentary from final answers.
        val commentary = BooleanArray(items.size)
        var nextIsStep = false
        for (index in items.indices.reversed()) {
            val item = items[index]
            val role = item.field("role").text()
            if (role == "assistant") {
                commentary[index] = nextIsStep && index <= liveFrom && item.field("resources").rows().isEmpty()
                if (item.field("text").text().isNotEmpty() || item.field("resources").rows().isNotEmpty()) nextIsStep = false
            } else if (role != "reasoning" && role != "file_changed") {
                nextIsStep = isStep(item)
            }
        }

        val result = mutableListOf<Group>()
        val pending = mutableListOf<JsonElement>()
        var start = 0
        fun flush() {
            if (pending.isEmpty()) return
            result += Group(start + indexOffset, pending.toList(), isProcess = true)
            pending.clear()
        }
        items.forEachIndexed { index, item ->
            val process = isStep(item) || item.field("role").text() in setOf("reasoning", "file_changed") || commentary[index]
            if (process) {
                if (pending.isEmpty()) start = index
                pending += item
            } else {
                flush()
                result += Group(index + indexOffset, listOf(item), isProcess = false)
            }
        }
        flush()
        return result
    }

    /** Rows for one detail level, pinned by contracts/native-conversations/v1/transcript-detail.json. */
    fun rows(items: List<JsonElement>, running: Boolean, detail: TranscriptDetail): List<TranscriptRow> {
        if (detail == TranscriptDetail.FULL) return items.indices.map { TranscriptRow.Item(it) }
        val standard = groups(items, running).map { group ->
            if (group.isProcess) TranscriptRow.Process(group.items.indices.map { group.id + it },
                group.toolCount, group.changedFileCount, group.failureCount)
            else TranscriptRow.Item(group.id)
        }
        if (detail == TranscriptDetail.STANDARD) return standard
        return standard.mapNotNull { row ->
            when (row) {
                is TranscriptRow.Process -> if (row.failures == 0) null else TranscriptRow.Failures(row.indices, row.failures)
                is TranscriptRow.Item -> row.takeIf { items[it.index].field("role").text() != "usage" }
                is TranscriptRow.Failures -> row
            }
        }
    }
}

/** Per-device presentation setting; every level reads the same snapshot items. */
enum class TranscriptDetail(val id: String, val label: String) {
    MINIMAL("minimal", "简洁"), STANDARD("standard", "标准"), FULL("full", "详细");
    companion object {
        val DEFAULT = STANDARD
        fun from(id: String?): TranscriptDetail = entries.firstOrNull { it.id == id } ?: DEFAULT
    }
}

/** Indices point into the snapshot items the rows were built from. */
sealed interface TranscriptRow {
    data class Item(val index: Int) : TranscriptRow
    data class Process(val indices: List<Int>, val tools: Int, val files: Int, val failures: Int) : TranscriptRow {
        /** Collapsed label, matching the iOS process group. */
        val summary: String get() = (listOf("执行过程") +
            listOfNotNull(tools.takeIf { it > 0 }?.let { "$it 步" }, files.takeIf { it > 0 }?.let { "$it 个文件" })).joinToString(" · ")
    }
    data class Failures(val indices: List<Int>, val count: Int) : TranscriptRow
}
