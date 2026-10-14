package science.wisp.remote

import kotlinx.serialization.json.*
import java.time.LocalDate

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

/** Marks each settled snapshot read once, like the iOS client, so the host clears 待处理 for replies seen here. */
class SeenMarker {
    private var last = ""
    suspend fun mark(service: RemoteService, snapshot: RemoteSnapshot) {
        if (snapshot.running || "native_conversation_seen" !in service.status.value.commands) return
        val stamp = "${snapshot.epoch}:${snapshot.route.project}:${snapshot.route.session}:${snapshot.sequence}"
        if (stamp == last) return
        service.call("native_conversation_seen", snapshot.route.project, snapshot.route.args)
        last = stamp
    }
}

/** The host's inbox already spans every project; the first project only routes the request. */
suspend fun loadInbox(call: suspend (command: String, project: String?) -> JsonElement): List<JsonElement> {
    val projects = call("list_projects", null)
    requireRemote(projects is JsonArray)
    val project = projects.rows().firstOrNull()?.field("id")?.text() ?: return emptyList()
    val rows = call("native_conversation_inbox", project)
    requireRemote(rows is JsonArray)
    return rows.rows()
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

data class LocalEntry(val id: String, val kind: String, val title: String, val text: String, val done: Boolean = false, val dueDate: LocalDate? = null)

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
        obj("id" to string(it.id), "kind" to string(it.kind), "title" to string(it.title), "text" to string(it.text), "done" to JsonPrimitive(it.done), "due_date" to (it.dueDate?.let { date -> string(date.toString()) } ?: JsonNull))
    })).toString()
    fun decode(text: String): List<LocalEntry> {
        val value = Json.parseToJsonElement(text)
        requireRemote(value.field("schema") == JsonPrimitive(1) && value.field("entries") is JsonArray)
        return value.field("entries").rows().map {
            requireRemote(it.field("id").text().isNotBlank() && it.field("kind").text() in setOf("checklist", "notebook"))
            LocalEntry(it.field("id").text(), it.field("kind").text(), it.field("title").text(), it.field("text").text(), it.field("done").flag(),
                it.field("due_date").let { date -> if (date == JsonNull) null else LocalDate.parse(date.text()) })
        }.also { requireRemote(it.map { row -> row.id }.distinct().size == it.size) }
    }
}

/** Stable device-local grouping; undated legacy entries are never silently assigned a deadline. */
enum class ChecklistSection(val label: String) {
    OVERDUE("逾期"), TODAY("今天"), UPCOMING("接下来"), UNDATED("未设日期"), COMPLETED("已完成")
}
fun LocalEntry.checklistSection(today: LocalDate): ChecklistSection = when {
    done -> ChecklistSection.COMPLETED
    dueDate == null -> ChecklistSection.UNDATED
    dueDate < today -> ChecklistSection.OVERDUE
    dueDate == today -> ChecklistSection.TODAY
    else -> ChecklistSection.UPCOMING
}

fun completedRuns(runs: List<JsonElement>): List<JsonElement> = runs.filter { it.field("status").text() == "succeeded" }
