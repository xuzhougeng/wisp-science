package science.wisp.remote

import kotlin.test.*
import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.test.*
import kotlinx.serialization.json.*

private class FakeSocket : RemoteSocket {
    val incoming = Channel<String>(Channel.UNLIMITED)
    val sent = mutableListOf<String>()
    var closed = false
    override var hostOffline = false
    override suspend fun receive(): String = incoming.receive()
    override fun send(text: String): Boolean { sent += text; return !closed }
    override fun close() { closed = true; incoming.close() }
}

@OptIn(ExperimentalCoroutinesApi::class)
class RemoteConnectionTest {
    @Test fun orderedRequestsUseNonceAndExactSequenceAndMatchResponses() = runTest {
        val socket = FakeSocket()
        socket.incoming.send(HostFrames.hello())
        val connection = RemoteConnection(backgroundScope, { socket })
        connection.connect(RemoteConfiguration.parse("https://relay.test", TEST_CODE))
        runCurrent()
        assertEquals(ConnectionPhase.ONLINE, connection.status.value.phase)
        val first = async { connection.call("list_projects") }
        val second = async { connection.call("list_projects") }
        runCurrent()
        val requests = socket.sent.map(HostFrames::open)
        assertEquals(listOf(1L, 2L), requests.map { it.field("seq").jsonPrimitive.long })
        assertTrue(requests.all { it.field("nonce").text() == "host-nonce" })
        socket.incoming.send(HostFrames.response(requests[1].field("id").text(), string("second")))
        socket.incoming.send(HostFrames.response(requests[0].field("id").text(), string("first")))
        assertEquals(string("first"), first.await())
        assertEquals(string("second"), second.await())
        assertFailsWith<RemoteFailure> { connection.call("unsupported") }
        assertEquals(2, socket.sent.size)
        connection.disconnect()
    }

    @Test fun timeoutAndReconnectNeverReplayMutationAndInvalidateReadSession() = runTest {
        val sockets = mutableListOf<FakeSocket>()
        val connection = RemoteConnection(backgroundScope, {
            FakeSocket().also { socket -> sockets += socket; socket.incoming.trySend(HostFrames.hello("nonce-${sockets.size}")) }
        }, requestTimeoutMs = 100, retryDelayMs = 10)
        connection.connect(RemoteConfiguration.parse("https://relay.test", TEST_CODE))
        runCurrent()
        val oldSession = connection.status.value.session
        val outcome = async { runCatching { connection.call("native_conversation_send", "project", obj("message" to string("once"))) } }
        runCurrent()
        advanceTimeBy(101); runCurrent()
        assertEquals(RemoteFailure.Reason.UNKNOWN, (outcome.await().exceptionOrNull() as RemoteFailure).reason)
        sockets.first().incoming.close()
        runCurrent()
        advanceTimeBy(11); runCurrent()
        assertEquals(2, sockets.size)
        assertNotEquals(oldSession, connection.status.value.session)
        assertEquals(1, sockets.first().sent.size)
        assertTrue(sockets.last().sent.isEmpty())
        connection.disconnect()
    }

    @Test fun disconnectResolvesPendingCallsAndNewConfigurationDropsOldResponses() = runTest {
        val first = FakeSocket().also { it.incoming.trySend(HostFrames.hello()) }
        val second = FakeSocket().also { it.incoming.trySend(HostFrames.hello("new")) }
        val sockets = ArrayDeque(listOf(first, second))
        val connection = RemoteConnection(backgroundScope, { sockets.removeFirst() })
        val config = RemoteConfiguration.parse("https://relay.test", TEST_CODE)
        connection.connect(config); runCurrent()
        val result = async { runCatching { connection.call("list_projects") } }
        runCurrent()
        connection.connect(config); runCurrent()
        assertEquals(RemoteFailure.Reason.UNKNOWN, (result.await().exceptionOrNull() as RemoteFailure).reason)
        assertTrue(first.closed)
        assertEquals(ConnectionPhase.ONLINE, connection.status.value.phase)
        assertTrue(second.sent.isEmpty())
        connection.disconnect()
    }

    @Test fun invalidHandshakeDoesNotAdvertiseCapabilities() = runTest {
        val socket = FakeSocket().also { it.incoming.trySend(HostFrames.seal(obj("type" to string("hello")))) }
        val connection = RemoteConnection(backgroundScope, { socket })
        connection.connect(RemoteConfiguration.parse("https://relay.test", TEST_CODE)); runCurrent()
        assertEquals(ConnectionPhase.RETRYING, connection.status.value.phase)
        assertTrue(connection.status.value.commands.isEmpty())
        connection.disconnect()
    }
}
