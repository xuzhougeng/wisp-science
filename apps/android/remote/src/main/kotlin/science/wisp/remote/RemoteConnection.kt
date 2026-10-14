package science.wisp.remote

import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.json.*
import okhttp3.*
import okio.ByteString

enum class ConnectionPhase { IDLE, CONNECTING, ONLINE, RETRYING, OFFLINE }
data class ConnectionStatus(
    val phase: ConnectionPhase = ConnectionPhase.IDLE,
    val session: String = "",
    val name: String = "",
    val commands: Set<String> = emptySet(),
)

interface RemoteService {
    val status: StateFlow<ConnectionStatus>
    suspend fun call(command: String, project: String? = null, args: JsonObject = obj()): JsonElement
    fun disconnect()
}

interface RemoteSocket {
    suspend fun receive(): String
    fun send(text: String): Boolean
    fun close()
    val hostOffline: Boolean
}

class OkHttpRemoteSocket(url: String) : RemoteSocket {
    private val frames = Channel<String>(8)
    @Volatile override var hostOffline = false
        private set
    private val socket = client.newWebSocket(Request.Builder().url(url).build(), object : WebSocketListener() {
        override fun onMessage(webSocket: WebSocket, text: String) {
            if (text.length > RemoteCrypto.HOST_LIMIT || !frames.trySend(text).isSuccess) {
                frames.close(RemoteFailure(RemoteFailure.Reason.FRAME))
                webSocket.cancel()
            }
        }
        override fun onMessage(webSocket: WebSocket, bytes: ByteString) {
            frames.close(RemoteFailure(RemoteFailure.Reason.FRAME))
            webSocket.cancel()
        }
        override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
            hostOffline = code == 4404
            frames.close(RemoteFailure(RemoteFailure.Reason.OFFLINE))
            webSocket.close(code, null)
        }
        override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
            // Transport exceptions may contain URLs. Never expose them or the link to the UI.
            frames.close(RemoteFailure(RemoteFailure.Reason.OFFLINE))
        }
    })
    override suspend fun receive(): String = frames.receive()
    override fun send(text: String): Boolean = socket.send(text)
    override fun close() { socket.cancel(); frames.cancel() }

    companion object {
        private val client = OkHttpClient.Builder()
            .followRedirects(false).followSslRedirects(false).retryOnConnectionFailure(false)
            .connectTimeout(15, TimeUnit.SECONDS).pingInterval(25, TimeUnit.SECONDS).build()
    }
}

/** Confined to the caller's scope dispatcher (Main in Android); socket callbacks only feed a channel. */
class RemoteConnection(
    private val scope: CoroutineScope,
    private val socketFactory: (String) -> RemoteSocket = ::OkHttpRemoteSocket,
    private val requestTimeoutMs: Long = 90_000,
    private val handshakeTimeoutMs: Long = 12_000,
    private val retryDelayMs: Long = 1_000,
) : RemoteService {
    private val mutableStatus = MutableStateFlow(ConnectionStatus())
    override val status: StateFlow<ConnectionStatus> = mutableStatus
    private var pump: Job? = null
    private var generation = UUID.randomUUID()
    private var socket: RemoteSocket? = null
    private var session: Session? = null
    private val lane = Mutex()
    private class Session(val socket: RemoteSocket, val crypto: RemoteCrypto, val nonce: String) {
        var sequence = 0L
        val pending = ConcurrentHashMap<String, CompletableDeferred<JsonElement>>()
        fun fail() {
            pending.values.forEach { it.completeExceptionally(RemoteFailure(RemoteFailure.Reason.UNKNOWN)) }
            pending.clear()
        }
    }

    fun connect(configuration: RemoteConfiguration) {
        disconnect()
        val expected = generation
        mutableStatus.value = ConnectionStatus(ConnectionPhase.CONNECTING)
        pump = scope.launch {
            var attempt = 0
            while (isActive && expected == generation) {
                var channel: RemoteSocket? = null
                try {
                    val activeSocket = socketFactory(configuration.webSocketUrl)
                    channel = activeSocket
                    socket = activeSocket
                    val hello = configuration.crypto.open(withTimeout(handshakeTimeoutMs) { activeSocket.receive() })
                    requireRemote(hello.field("type").text() == "hello" && hello.field("nonce").text().isNotEmpty() &&
                        hello.field("commands") is JsonArray && hello.field("commands").rows().all { it.text().isNotEmpty() })
                    val current = Session(activeSocket, configuration.crypto, hello.field("nonce").text())
                    session = current
                    mutableStatus.value = ConnectionStatus(ConnectionPhase.ONLINE, UUID.randomUUID().toString(),
                        hello.field("name").text(), hello.field("commands").rows().map { it.text() }.toSet())
                    attempt = 0
                    while (isActive && expected == generation) {
                        val message = configuration.crypto.open(activeSocket.receive())
                        requireRemote(message.field("type").text() == "response")
                        val response = message.field("response")
                        requireRemote(response.field("schema").text() == "wisp.native-settings.v1" && response.field("id").text().isNotEmpty())
                        current.pending.remove(response.field("id").text())?.let { pending ->
                            if (response.field("error") != JsonNull) pending.completeExceptionally(RemoteFailure(RemoteFailure.Reason.REJECTED))
                            else pending.complete(response.field("result"))
                        }
                    }
                } catch (error: CancellationException) {
                    if (error !is TimeoutCancellationException) throw error
                } catch (_: Exception) {
                    // A retry refreshes reads only. It never retains or resends requests.
                } finally {
                    channel?.close()
                    if (expected == generation) {
                        session?.fail(); session = null; socket = null
                    }
                }
                if (expected != generation) return@launch
                mutableStatus.value = ConnectionStatus(if (channel?.hostOffline == true) ConnectionPhase.OFFLINE else ConnectionPhase.RETRYING)
                delay(retryDelayMs * (1L shl attempt.coerceAtMost(3)))
                attempt = (attempt + 1).coerceAtMost(3)
            }
        }
    }

    override suspend fun call(command: String, project: String?, args: JsonObject): JsonElement {
        currentCoroutineContext().ensureActive()
        val current = session ?: throw RemoteFailure(RemoteFailure.Reason.OFFLINE)
        val id = UUID.randomUUID().toString()
        val pending = CompletableDeferred<JsonElement>()
        try {
            lane.withLock {
                requireRemote(current === session, RemoteFailure.Reason.OFFLINE)
                requireRemote(command in status.value.commands, RemoteFailure.Reason.UNSUPPORTED)
                requireRemote(current.pending.size < 64, RemoteFailure.Reason.BUSY)
                val frame = current.crypto.seal(obj(
                    "nonce" to string(current.nonce), "seq" to JsonPrimitive(++current.sequence),
                    "id" to string(id), "command" to string(command),
                    "project_id" to (project?.let(::string) ?: JsonNull), "args" to args,
                ))
                current.pending[id] = pending
                if (!current.socket.send(frame)) {
                    current.socket.close()
                    throw RemoteFailure(RemoteFailure.Reason.UNKNOWN)
                }
            }
            return withTimeout(requestTimeoutMs) { pending.await() }
        } catch (_: TimeoutCancellationException) {
            throw RemoteFailure(RemoteFailure.Reason.UNKNOWN)
        } finally {
            current.pending.remove(id)
        }
    }

    override fun disconnect() {
        generation = UUID.randomUUID()
        pump?.cancel(); pump = null
        socket?.close(); socket = null
        session?.fail(); session = null
        mutableStatus.value = ConnectionStatus()
    }
}
