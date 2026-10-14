package science.wisp.remote

import java.net.URI
import java.util.Base64
import kotlinx.serialization.json.*
import org.commonmark.node.Node
import org.commonmark.parser.Parser
import org.commonmark.ext.gfm.tables.TablesExtension
import org.commonmark.ext.gfm.strikethrough.StrikethroughExtension
import org.commonmark.ext.autolink.AutolinkExtension
import org.commonmark.ext.task.list.items.TaskListItemsExtension

/** CommonMark AST, rendered by native Compose components. No HTML/WebView execution. */
object Markdown {
    private val parser = Parser.builder().extensions(listOf(TablesExtension.create(), StrikethroughExtension.create(),
        AutolinkExtension.create(), TaskListItemsExtension.create())).build()
    fun parse(text: String): Node = parser.parse(text)

    sealed interface Target {
        data class ProjectFile(val path: String) : Target
        data class Website(val url: String) : Target
    }

    fun target(reference: String, directory: String = "."): Target? {
        val uri = runCatching { URI(reference.trim().replace(" ", "%20")) }.getOrNull() ?: return null
        if (uri.scheme?.lowercase() in setOf("https", "http")) {
            return if (!uri.host.isNullOrBlank() && uri.userInfo == null) Target.Website(uri.toASCIIString()) else null
        }
        if (uri.scheme != null || uri.rawAuthority != null || reference.startsWith("//")) return null
        val path = uri.path.orEmpty()
        if (path.isBlank() || path.any { it.isISOControl() } || '\\' in path || ':' in path || path.startsWith("//")) return null
        val absolute = path.startsWith('/')
        val parts = (if (absolute || directory == ".") emptyList() else directory.split('/').filter { it.isNotEmpty() }).toMutableList()
        for (part in path.split('/')) when (part) {
            "", "." -> Unit
            ".." -> if (parts.isEmpty()) return null else parts.removeAt(parts.lastIndex)
            else -> parts += part
        }
        if (parts.isEmpty()) return null
        return Target.ProjectFile((if (absolute || directory.startsWith('/')) "/" else "") + parts.joinToString("/"))
    }

    fun imageSource(reference: String, resources: List<JsonElement>, directory: String = "."): JsonObject? {
        // A bound image must keep its captured version, including when that version is unavailable.
        resources.firstOrNull { it.field("originalReference").text() == reference }?.let {
            return obj("resource_id" to it.field("id"))
        }
        val path = (target(reference, directory) as? Target.ProjectFile)?.path ?: return null
        return obj("path" to string(path))
    }

    fun isImage(path: String): Boolean = path.substringAfterLast('.', "").lowercase() in setOf("png", "jpg", "jpeg", "webp", "gif", "bmp", "tif", "tiff")
    fun isDocument(path: String): Boolean = path.substringAfterLast('.', "").lowercase() in setOf("md", "markdown", "mdown")
}

fun Node.children(): List<Node> = generateSequence(firstChild) { it.next }.toList()

object ImagePreview {
    private const val MAX_BYTES = 6 * 1024 * 1024
    fun bytes(value: JsonElement): ByteArray {
        val encoded = value.field("base64").text()
        val mime = value.field("mime").text().ifBlank { value.field("mime_type").text() }
        requireRemote(mime in setOf("image/png", "image/jpeg", "image/webp", "image/gif") && encoded.isNotEmpty() && encoded.length <= MAX_BYTES * 4 / 3,
            RemoteFailure.Reason.PROTOCOL)
        val bytes = try { Base64.getDecoder().decode(encoded) } catch (_: IllegalArgumentException) { throw RemoteFailure(RemoteFailure.Reason.PROTOCOL) }
        requireRemote(bytes.size <= MAX_BYTES)
        return bytes
    }
}
