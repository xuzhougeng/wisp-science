package science.wisp.remote

import java.security.MessageDigest
import java.util.Base64
import javax.crypto.Cipher
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.SecretKeySpec
import kotlin.test.*
import kotlinx.serialization.json.*

internal const val TEST_CODE = "0001-0203-0405-0607-0809-0a0b-0c0d-0e0f"
internal fun fixture(path: String): JsonElement = Json.parseToJsonElement(
    requireNotNull(object {}.javaClass.classLoader.getResourceAsStream(path)).bufferedReader().use { it.readText() })

/** Host fixture uses its own key derivation/direction, independent of RemoteCrypto. */
internal object HostFrames {
    private val key = SecretKeySpec(MessageDigest.getInstance("SHA-256").digest(
        "wisp-remote/key/v1".toByteArray() + ByteArray(16) { it.toByte() }), "AES")
    fun seal(value: JsonElement): String {
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, key)
        cipher.updateAAD("wisp-remote/v1/h2c".toByteArray())
        return Base64.getEncoder().encodeToString(cipher.iv + cipher.doFinal(value.toString().toByteArray()))
    }
    fun open(frame: String): JsonElement {
        val bytes = Base64.getDecoder().decode(frame)
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(128, bytes.copyOfRange(0, 12)))
        cipher.updateAAD("wisp-remote/v1/c2h".toByteArray())
        return Json.parseToJsonElement(String(cipher.doFinal(bytes.copyOfRange(12, bytes.size))))
    }
    fun hello(nonce: String = "host-nonce") = seal(obj("type" to string("hello"), "nonce" to string(nonce),
        "name" to string("Test host"), "commands" to JsonArray(listOf("list_projects", "native_conversation_send").map(::string))))
    fun response(id: String, result: JsonElement) = seal(obj("type" to string("response"), "response" to obj(
        "schema" to string("wisp.native-settings.v1"), "id" to string(id), "result" to result, "error" to JsonNull)))
}

class ProtocolTest {
    @Test fun linksPreserveProxyPrefixAndNeverTransmitTheCode() {
        val config = RemoteConfiguration.parse("https://relay.example.test/wisp/remote#$TEST_CODE/p/project/s/session")
        assertEquals("https://relay.example.test/wisp/", config.relay.toString())
        assertEquals("cf4778d13d24d0dd1313bca1709267bb", config.crypto.sid)
        assertEquals("wss://relay.example.test/wisp/v1/remote/client/cf4778d13d24d0dd1313bca1709267bb", config.webSocketUrl)
        assertFalse(config.webSocketUrl.contains(config.code))
        assertFalse(config.toString().contains(config.code))
    }
    @Test fun malformedOrInsecureLinksAreRejected() {
        listOf("http://relay.test", "http://127.0.0.1:8787", "wss://relay.test", "https://user:password@relay.test",
            "https://relay.test?code=secret", "not a URL", "https://relay.test:0", "https://relay.test:65536").forEach {
            assertFailsWith<RemoteFailure>(it) { RemoteConfiguration.parse(it, TEST_CODE) }
        }
        listOf("", "abcd", "zz" + TEST_CODE, "0001/0203/0405/0607/0809/0a0b/0c0d/0e0f").forEach {
            assertFailsWith<RemoteFailure> { RemoteConfiguration.parse("https://relay.test", it) }
        }
    }
    @Test fun framesMatchRustVectorAndAreDirectionBound() {
        val crypto = RemoteCrypto(TEST_CODE)
        // Fixed WebCrypto vector used by crates/wisp-sync/src/remote.rs and the iOS tests.
        assertEquals(obj("ping" to JsonPrimitive(1)), HostFrames.open("AAECAwQFBgcICQoL9OOvsOOON1awkl3qncEpESEISdpba7HB8iw="))
        val value = obj("text" to string("研究数据"))
        assertEquals(value, crypto.open(HostFrames.seal(value)))
        assertEquals(value, HostFrames.open(crypto.seal(value)))
        assertFailsWith<RemoteFailure> { crypto.open(crypto.seal(value)) }
        val bytes = Base64.getDecoder().decode(HostFrames.seal(value))
        bytes[bytes.lastIndex] = (bytes.last().toInt() xor 1).toByte()
        assertFailsWith<RemoteFailure> { crypto.open(Base64.getEncoder().encodeToString(bytes)) }
        assertFailsWith<RemoteFailure> { RemoteCrypto("f".repeat(32)).open(HostFrames.seal(value)) }
        assertFailsWith<RemoteFailure> { crypto.open("invalid") }
        assertEquals(RemoteFailure.Reason.TOO_LARGE, assertFailsWith<RemoteFailure> { crypto.seal(string("x".repeat(800_000))) }.reason)
    }
    @Test fun sharedSnapshotContractKeepsLargeQueueIdsAndRejectsWrongRoutes() {
        val route = ConversationRoute("project-a", "session-a")
        val data = fixture("native-conversations/v1/snapshot-images-queue.json")
        val snapshot = RemoteSnapshot(data, route)
        assertEquals(listOf("18446744073709551615", "9007199254740993"), snapshot.value.field("queue").field("items").rows().map { it.field("id").text() })
        assertFailsWith<RemoteFailure> { RemoteSnapshot(data, route.copy(project = "other")) }
        val full = fixture("native-conversations/v1/snapshot.json").jsonObject
        assertFailsWith<RemoteFailure> { RemoteSnapshot(JsonObject(full + ("approvals" to JsonArray(listOf(
            obj("frame_id" to string("other"), "approval_id" to string("approval")))))), route) }
    }
    @Test fun mutationsRequireTheCurrentConnectionAndWritableSnapshot() {
        val route = ConversationRoute("project-a", "session-a")
        val data = fixture("native-conversations/v1/snapshot.json").jsonObject
        val snapshot = RemoteSnapshot(data, route)
        val status = ConnectionStatus(ConnectionPhase.ONLINE, "new-socket", commands = setOf("send"))
        assertTrue(canAct(status, snapshot, "new-socket", "send"))
        assertFalse(canAct(status, snapshot, "old-socket", "send"))
        assertFalse(canAct(status.copy(phase = ConnectionPhase.OFFLINE), snapshot, "new-socket", "send"))
        assertFalse(canAct(status, snapshot, "new-socket", "unsupported"))
        assertFalse(canAct(status, RemoteSnapshot(JsonObject(data + ("read_only" to JsonPrimitive(true))), route), "new-socket", "send"))
        val older = RemoteSnapshot(JsonObject(data + ("sequence" to JsonPrimitive(1))), route)
        assertFalse(older.supersedes(snapshot))
    }
    @Test fun offlineNotesRoundTripAndUnknownSchemasDoNotOverwriteData() {
        val rows = listOf(LocalEntry("1", "notebook", "观察", "实验记录\n第二行"), LocalEntry("2", "checklist", "复核", "样本", true))
        assertEquals(rows, LocalEntries.decode(LocalEntries.encode(rows)))
        assertFailsWith<RemoteFailure> { LocalEntries.decode("{\"schema\":2,\"entries\":[]}") }
        assertFailsWith<RemoteFailure> { LocalEntries.decode(LocalEntries.encode(rows + rows.first())) }
        assertFalse(UnavailableTextRecognizer.available)
    }
}
