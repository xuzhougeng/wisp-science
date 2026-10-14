package science.wisp.remote

import java.net.URI
import java.security.MessageDigest
import java.security.SecureRandom
import java.util.Base64
import javax.crypto.Cipher
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.SecretKeySpec
import kotlinx.serialization.json.*

class RemoteFailure(val reason: Reason) : Exception(reason.message) {
    enum class Reason(val message: String) {
        ADDRESS("请输入不含账号或查询参数的 HTTPS 中继地址。"),
        CODE("联机码应为 32 位十六进制字符，可含空格和短横线。"),
        FRAME("加密消息验证失败，请检查联机码和主机版本。"),
        PROTOCOL("主机返回的数据不兼容。"),
        UNSUPPORTED("当前主机不支持此功能，请更新主机。"),
        OFFLINE("连接不可用，请保持 Wisp 主机在线。"),
        UNKNOWN("操作结果未确认；请刷新或到主机核对，应用不会自动重发。"),
        TOO_LARGE("消息过大，请减少内容。"),
        REJECTED("主机未接受此请求，请在主机查看详情。"),
        BUSY("请求过多，请稍后重试。"),
    }
}

internal fun requireRemote(value: Boolean, reason: RemoteFailure.Reason = RemoteFailure.Reason.PROTOCOL) {
    if (!value) throw RemoteFailure(reason)
}

fun JsonElement.field(key: String): JsonElement = (this as? JsonObject)?.get(key) ?: JsonNull
fun JsonElement.text(): String = (this as? JsonPrimitive)?.takeIf { it.isString }?.content.orEmpty()
fun JsonElement.flag(): Boolean = (this as? JsonPrimitive)?.booleanOrNull ?: false
fun JsonElement.rows(): List<JsonElement> = (this as? JsonArray)?.toList().orEmpty()
fun obj(vararg pairs: Pair<String, JsonElement>): JsonObject = JsonObject(mapOf(*pairs))
fun string(value: String): JsonPrimitive = JsonPrimitive(value)

/** Never data-class this secret-bearing object: default toString must not expose the code. */
class RemoteConfiguration private constructor(val relay: URI, val code: String) {
    val crypto = RemoteCrypto(code)
    val webSocketUrl: String = relay.resolve("v1/remote/client/${crypto.sid}").toASCIIString()
        .replaceFirst("https:", "wss:")

    companion object {
        fun parse(address: String, code: String = ""): RemoteConfiguration {
            val uri = try { URI(address.trim()) } catch (_: Exception) {
                throw RemoteFailure(RemoteFailure.Reason.ADDRESS)
            }
            requireRemote(uri.scheme.equals("https", true) && !uri.host.isNullOrBlank() &&
                uri.rawUserInfo == null && uri.rawQuery == null &&
                (uri.port == -1 || uri.port in 1..65535), RemoteFailure.Reason.ADDRESS)
            val normalized = normalizeCode(code.trim().ifEmpty { uri.fragment.orEmpty().substringBefore('/') })
            val path = uri.rawPath.orEmpty().trimEnd('/').removeSuffix("/remote") + "/"
            val base = URI("https://${uri.rawAuthority}$path")
            return RemoteConfiguration(base, normalized)
        }

        fun normalizeCode(text: String): String {
            val code = text.lowercase().filterNot { it == '-' || it.isWhitespace() }
            requireRemote(code.length == 32 && code.all { it in '0'..'9' || it in 'a'..'f' }, RemoteFailure.Reason.CODE)
            return code
        }
    }
}

/** Shared Rust/WebCrypto protocol, implemented with JCA instead of Apple's CryptoKit. */
class RemoteCrypto(normalizedCode: String) {
    private val secret = RemoteConfiguration.normalizeCode(normalizedCode).chunked(2).map { it.toInt(16).toByte() }.toByteArray()
    private fun derive(label: String) = MessageDigest.getInstance("SHA-256").digest(label.toByteArray() + secret)
    val sid = derive("wisp-remote/sid/v1").take(16).joinToString("") { "%02x".format(it) }
    private val key = SecretKeySpec(derive("wisp-remote/key/v1"), "AES")

    fun seal(value: JsonElement): String {
        val bytes = value.toString().toByteArray(Charsets.UTF_8)
        requireRemote(((bytes.size.toLong() + 28 + 2) / 3) * 4 <= CLIENT_LIMIT, RemoteFailure.Reason.TOO_LARGE)
        val iv = ByteArray(12).also { SecureRandom().nextBytes(it) }
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, key, GCMParameterSpec(128, iv))
        cipher.updateAAD("wisp-remote/v1/c2h".toByteArray())
        return Base64.getEncoder().encodeToString(iv + cipher.doFinal(bytes))
    }

    fun open(frame: String): JsonElement = try {
        requireRemote(frame.length <= HOST_LIMIT, RemoteFailure.Reason.FRAME)
        val bytes = Base64.getDecoder().decode(frame)
        requireRemote(bytes.size >= 28, RemoteFailure.Reason.FRAME)
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(128, bytes.copyOfRange(0, 12)))
        cipher.updateAAD("wisp-remote/v1/h2c".toByteArray())
        Json.parseToJsonElement(String(cipher.doFinal(bytes.copyOfRange(12, bytes.size)), Charsets.UTF_8))
    } catch (_: Exception) { throw RemoteFailure(RemoteFailure.Reason.FRAME) }

    companion object {
        const val CLIENT_LIMIT = 1024 * 1024
        const val HOST_LIMIT = 16 * 1024 * 1024
    }
}
