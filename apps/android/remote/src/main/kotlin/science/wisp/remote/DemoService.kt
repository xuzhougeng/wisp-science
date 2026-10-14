package science.wisp.remote

import java.util.UUID
import java.time.LocalDate
import java.time.ZoneId
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.serialization.json.*

/** Has no transport or credential dependency. Assets are the iOS client's synthetic dataset. */
class DemoService(manifest: String, private val readImageAsset: ((String) -> ByteArray)? = null, private val readAsset: (String) -> String) : RemoteService {
    private val data = Json.parseToJsonElement(manifest)
    private val project = data.field("project")
    private val sessions = data.field("sessions").rows()
    private val additions = mutableMapOf<String, List<JsonElement>>()
    private val resolved = mutableSetOf<String>()
    private val seen = mutableSetOf<String>()
    private val researchDay = LocalDate.now()
    private var sequence = 1L
    private val mutableStatus = MutableStateFlow(ConnectionStatus(ConnectionPhase.ONLINE, UUID.randomUUID().toString(),
        "离线演示 · 合成数据", setOf("list_projects", "remote_sessions", "native_conversation_snapshot",
            "native_research_calendar", "native_research_journey", "native_conversation_inbox", "native_conversation_image",
            "native_conversation_seen", "native_conversation_send", "native_conversation_approve", "native_conversation_panel_file_directory", "native_conversation_panel_file_read")))
    override val status: StateFlow<ConnectionStatus> = mutableStatus

    override suspend fun call(command: String, project: String?, args: JsonObject): JsonElement {
        requireRemote(status.value.phase == ConnectionPhase.ONLINE, RemoteFailure.Reason.OFFLINE)
        requireRemote(command in status.value.commands, RemoteFailure.Reason.UNSUPPORTED)
        if (command == "list_projects") return JsonArray(listOf(JsonObject(this.project.jsonObject + mapOf(
            "session_count" to JsonPrimitive(sessions.size), "needs_you_count" to JsonPrimitive(sessionRows().count { it.field("status").text() == "needs_you" })))))
        if (command == "native_research_calendar") {
            val ids = args.field("project_ids").rows()
            requireRemote(project == null && args.field("project_ids") is JsonArray && ids.all { it.text() == this.project.field("id").text() })
            return JsonArray(ids.distinct().map { obj("project_id" to it, "history" to history(args), "error" to JsonNull) })
        }
        requireRemote(project == this.project.field("id").text())
        if (command == "remote_sessions") return JsonArray(sessionRows())
        if (command == "native_conversation_inbox") return JsonArray(sessionRows().filter { it.field("status").text() == "needs_you" })
        if (command == "native_research_journey") return history(args)
        val id = args.field("session_id").text()
        val session = sessions.firstOrNull { it.field("id").text() == id } ?: throw RemoteFailure(RemoteFailure.Reason.PROTOCOL)
        return when (command) {
            "native_conversation_snapshot" -> obj(
                "schema" to string("wisp.native-conversations.v1"), "project_id" to string(project!!), "session_id" to string(id),
                "epoch" to string(status.value.session), "sequence" to JsonPrimitive(sequence),
                "running" to JsonPrimitive(false), "stopping" to JsonPrimitive(false), "read_only" to JsonPrimitive(false),
                "items" to JsonArray(session.field("items").rows() + additions[id].orEmpty()),
                "approvals" to JsonArray(session.field("approvals").rows().filterNot { it.field("approval_id").text() in resolved }),
                "run_cards" to session.field("run_cards"), "follow_ups" to session.field("follow_ups"),
            )
            "native_conversation_send" -> {
                val message = args.field("message").text()
                requireRemote(message.isNotBlank())
                additions[id] = additions[id].orEmpty() + listOf(
                    obj("role" to string("user"), "text" to string(message)),
                    obj("role" to string("assistant"), "text" to string("本地预设回复：已收到你的演示消息。可以打开项目文件阅读报告；没有调用 AI 或执行计算。")),
                )
                sequence++
                JsonNull
            }
            "native_conversation_approve" -> {
                val approval = args.field("approval_id").text()
                requireRemote(session.field("approvals").rows().any { it.field("approval_id").text() == approval } && approval !in resolved)
                requireRemote(args.field("scope").text() == "once")
                resolved += approval
                additions[id] = additions[id].orEmpty() + obj("role" to string("assistant"), "text" to string(
                    if (args.field("approved").flag()) "演示审批已批准；没有执行命令。" else "演示审批已拒绝；没有执行命令。"))
                sequence++
                JsonNull
            }
            "native_conversation_seen" -> {
                seen += id
                JsonNull
            }
            "native_conversation_panel_file_directory" -> {
                val path = args.field("path").text()
                requireRemote(path == "." || validPath(path))
                val prefix = if (path == ".") "" else "$path/"
                val entries = data.field("files").rows().map { it.field("path").text() }.filter { it.startsWith(prefix) }
                    .map { it.removePrefix(prefix) }.groupBy { it.substringBefore('/') }.map { (name, paths) ->
                        obj("name" to string(name), "is_dir" to JsonPrimitive(paths.any { '/' in it }))
                    }
                obj("entries" to JsonArray(entries))
            }
            "native_conversation_image" -> {
                val path = args.field("path").text()
                requireRemote(args.field("resource_id") == JsonNull && validPath(path) && path.endsWith(".png") &&
                    data.field("files").rows().any { it.field("path").text() == path })
                val bytes = readImageAsset?.invoke("files/$path") ?: throw RemoteFailure(RemoteFailure.Reason.UNSUPPORTED)
                obj("base64" to string(java.util.Base64.getEncoder().encodeToString(bytes)), "mime" to string("image/png"))
            }
            "native_conversation_panel_file_read" -> {
                val path = args.field("path").text()
                requireRemote(validPath(path) && data.field("files").rows().any { it.field("path").text() == path })
                val text = if (path.endsWith(".png")) "Android 框架版暂不显示图片，请阅读同目录的 qc_report.md。" else readAsset("files/$path")
                obj("content" to obj("text" to string(text), "truncated" to JsonPrimitive(false)))
            }
            else -> throw RemoteFailure(RemoteFailure.Reason.UNSUPPORTED)
        }
    }
    private fun sessionRows() = sessions.mapIndexed { index, session ->
        val items = session.field("items").rows() + additions[session.field("id").text()].orEmpty()
        val pending = session.field("approvals").rows().any { it.field("approval_id").text() !in resolved } ||
            items.withIndex().any { (i, item) -> Question.parse(item) != null && items.drop(i + 1).none { it.field("role").text() == "user" } }
        obj("id" to session.field("id"), "title" to session.field("title"), "project_id" to project.field("id"),
            "project_name" to project.field("name"), "status" to string(if (pending || session.field("id").text() !in seen) "needs_you" else "idle"),
            "activity_at" to JsonPrimitive(researchDay.minusDays(index.toLong()).atTime(10, 0).atZone(ZoneId.systemDefault()).toEpochSecond()))
    }
    private fun history(args: JsonObject): JsonObject {
        val from = (args.field("from") as? JsonPrimitive)?.longOrNull
        val until = (args.field("until") as? JsonPrimitive)?.longOrNull
        requireRemote(from != null && until != null && from >= 0 && until > from && until - from <= 32 * 86400)
        val entries = sessions.mapIndexedNotNull { index, session ->
            val stamp = researchDay.minusDays(index.toLong()).atTime(10, 0).atZone(ZoneId.systemDefault()).toEpochSecond()
            if (stamp < from!! || stamp >= until!!) null else obj(
                "id" to string("research-" + session.field("id").text()), "kind" to string("note"),
                "title" to session.field("title"), "summary" to string("本地演示 · 合成样本的研究记录。可打开关联对话查看报告和流程。"),
                "occurred_at" to JsonPrimitive(stamp), "recorded_at" to JsonPrimitive(stamp),
                "frame_id" to session.field("id"), "source_discarded" to JsonPrimitive(false), "status" to string("recorded"))
        }
        return obj("entries" to JsonArray(entries), "truncated" to JsonPrimitive(false))
    }
    override fun disconnect() { mutableStatus.value = ConnectionStatus() }
    private fun validPath(path: String) = path.isNotBlank() && !path.startsWith('/') && '\\' !in path &&
        path.split('/').all { it.isNotEmpty() && it != "." && it != ".." }
}
