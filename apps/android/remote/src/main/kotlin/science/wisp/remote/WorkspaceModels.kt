package science.wisp.remote

import kotlinx.serialization.json.*

data class ConversationRoute(val project: String, val session: String) {
    val args: JsonObject get() = obj("session_id" to string(session))
}

/** Projection of wisp-dto, validated against contracts/native-conversations/v1. */
class RemoteSnapshot(val value: JsonElement, val route: ConversationRoute) {
    val epoch = value.field("epoch").text()
    val sequence = (value.field("sequence") as? JsonPrimitive)?.content?.toULongOrNull()
    val items = value.field("items").rows()
    val running = value.field("running").flag()
    val writable = !value.field("read_only").flag() && value.field("acp_agent_id").text().isEmpty()
    init {
        requireRemote(value.field("schema").text() == "wisp.native-conversations.v1" &&
            value.field("project_id").text() == route.project && value.field("session_id").text() == route.session &&
            epoch.isNotEmpty() && sequence != null && sequence > 0u &&
            value.field("items") is JsonArray && value.field("approvals") is JsonArray &&
            listOf("running", "read_only", "stopping").all { value.field(it).let { v -> v == JsonPrimitive(true) || v == JsonPrimitive(false) } } &&
            value.field("approvals").rows().all { it.field("frame_id").text() == route.session && it.field("approval_id").text().isNotEmpty() } &&
            value.field("queue").field("items").rows().all { it.field("id").text().isNotEmpty() && it.field("digest").text().isNotEmpty() })
    }
    fun supersedes(previous: RemoteSnapshot?): Boolean = previous == null || epoch != previous.epoch || sequence!! >= previous.sequence!!
}

/** All mutations require a snapshot fetched during this authenticated socket session. */
fun canAct(status: ConnectionStatus, snapshot: RemoteSnapshot?, snapshotSession: String, command: String): Boolean =
    status.phase == ConnectionPhase.ONLINE && status.session.isNotEmpty() && status.session == snapshotSession &&
        command in status.commands && snapshot?.writable == true

data class Question(val text: String, val options: List<Pair<String, String>>, val freeform: Boolean) {
    companion object {
        fun parse(item: JsonElement): Question? {
            if (item.field("role").text() != "question") return null
            val value = runCatching { Json.parseToJsonElement(item.field("text").text()) }.getOrNull() ?: return null
            if (value.field("source").text() != "native" || value.field("question").text().isBlank()) return null
            return Question(value.field("question").text(), value.field("options").rows().map {
                it.field("label").text() to it.field("description").text()
            }, value.field("allow_freeform").flag())
        }
    }
}

data class LocalEntry(val id: String, val kind: String, val title: String, val text: String, val done: Boolean = false)

/** Text-only offline foundation. Photo recognition is deliberately an unsupported platform capability. */
interface NotebookTextRecognizer {
    val available: Boolean
    suspend fun recognize(image: ByteArray): String
}
object UnavailableTextRecognizer : NotebookTextRecognizer {
    override val available = false
    override suspend fun recognize(image: ByteArray): String = throw RemoteFailure(RemoteFailure.Reason.UNSUPPORTED)
}

object LocalEntries {
    fun encode(entries: List<LocalEntry>): String = obj("schema" to JsonPrimitive(1), "entries" to JsonArray(entries.map {
        obj("id" to string(it.id), "kind" to string(it.kind), "title" to string(it.title), "text" to string(it.text), "done" to JsonPrimitive(it.done))
    })).toString()
    fun decode(text: String): List<LocalEntry> {
        val value = Json.parseToJsonElement(text)
        requireRemote(value.field("schema") == JsonPrimitive(1) && value.field("entries") is JsonArray)
        return value.field("entries").rows().map {
            requireRemote(it.field("id").text().isNotBlank() && it.field("kind").text() in setOf("checklist", "notebook"))
            LocalEntry(it.field("id").text(), it.field("kind").text(), it.field("title").text(), it.field("text").text(), it.field("done").flag())
        }.also { requireRemote(it.map { row -> row.id }.distinct().size == it.size) }
    }
}
