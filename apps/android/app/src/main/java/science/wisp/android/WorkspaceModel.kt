package science.wisp.android

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import java.time.YearMonth
import java.time.ZoneId
import java.util.UUID
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.*
import kotlinx.serialization.json.*
import science.wisp.remote.*

enum class PageKind { PROJECTS, CONNECTION, SESSIONS, CONVERSATION, FILES, JOURNEY, CHECKLIST, NOTEBOOK, EDITOR }
data class Page(
    val kind: PageKind, val title: String, val project: String = "", val session: String = "",
    val path: String = ".", val directory: Boolean = true, val entryKind: String = "notebook",
) {
    val route get() = ConversationRoute(project, session)
}
data class WorkspaceState(
    val stack: List<Page> = listOf(Page(PageKind.PROJECTS, "Wisp Science")),
    val status: ConnectionStatus = ConnectionStatus(), val demo: Boolean = false,
    val projects: List<JsonElement> = emptyList(), val sessions: List<JsonElement> = emptyList(),
    val snapshot: RemoteSnapshot? = null, val snapshotSession: String = "",
    val detail: JsonElement = JsonNull, val entries: List<LocalEntry> = emptyList(),
    val drafts: Map<ConversationRoute, String> = emptyMap(),
    val uncertain: Set<ConversationRoute> = emptySet(),
    val month: YearMonth = YearMonth.now(),
    val loading: Boolean = false, val busy: Boolean = false, val localBusy: Boolean = true,
    val error: String? = null, val localError: String? = null,
) {
    val page get() = stack.last()
    fun can(command: String): Boolean = !busy && page.route !in uncertain &&
        canAct(status, snapshot?.takeIf { it.route == page.route }, snapshotSession, command)
}

class WorkspaceModel(application: Application) : AndroidViewModel(application) {
    private val mutable = MutableStateFlow(WorkspaceState())
    val state = mutable.asStateFlow()
    private val credentials = DeviceCredentials(application)
    private val localStore = DeviceEntries(application)
    private val preferences = application.getSharedPreferences("workspace", 0)
    private val remote = RemoteConnection(viewModelScope)
    private var service: RemoteService = remote
    private var configuration: RemoteConfiguration? = null
    private var savedConnection = ""
    private var watcher: Job? = null
    private var reader: Job? = null
    private var polling: Job? = null
    private var revision = 0L
    private var workspaceEpoch = 0L
    private var foreground = false

    init {
        viewModelScope.launch {
            try {
                val entries = withContext(Dispatchers.IO) { localStore.load() }
                mutable.update { it.copy(entries = entries) }
            }
            catch (_: Exception) { mutable.update { it.copy(localError = "本机记录读取失败。为保护已有内容，暂时停止写入。") } }
            finally { mutable.update { it.copy(localBusy = false) } }
        }
        if (preferences.getBoolean("demo", false)) enterDemo()
        else {
            try { configuration = credentials.load() }
            catch (_: Exception) { mutable.update { it.copy(error = "已保存的连接无法解密，请重新连接或忘记连接。") } }
            observe(remote)
        }
    }

    private fun observe(next: RemoteService) {
        watcher?.cancel()
        service = next
        watcher = viewModelScope.launch {
            next.status.collect { status ->
                if (service !== next) return@collect
                mutable.update { it.copy(status = status) }
                if (status.phase == ConnectionPhase.ONLINE) {
                    if (next === remote) configuration?.let { config ->
                        val identity = config.relay.toString() + config.crypto.sid
                        if (identity != savedConnection) {
                            try { credentials.save(config); savedConnection = identity }
                            catch (_: Exception) { mutable.update { it.copy(error = "已连接，但设备未能安全保存联机码。下次启动需要重新输入。") } }
                        }
                    }
                    refresh()
                }
            }
        }
    }

    fun resume() {
        if (foreground) return
        foreground = true
        if (state.value.demo) refresh() else configuration?.let(remote::connect)
        polling = viewModelScope.launch {
            while (isActive) { delay(3_000); if (!state.value.loading) refresh() }
        }
    }
    fun pause() {
        foreground = false
        polling?.cancel(); polling = null
        reader?.cancel(); reader = null
        mutable.update { it.copy(loading = false, snapshotSession = "") }
        if (!state.value.demo) remote.disconnect()
    }

    private fun reset(demo: Boolean) {
        revision++
        workspaceEpoch++
        reader?.cancel()
        service.disconnect()
        mutable.update { WorkspaceState(entries = it.entries, localError = it.localError, localBusy = it.localBusy, demo = demo) }
    }
    fun connect(address: String, code: String) {
        try {
            val config = RemoteConfiguration.parse(address, code)
            reset(false)
            configuration = config
            preferences.edit().putBoolean("demo", false).apply()
            observe(remote)
            remote.connect(config)
        } catch (error: RemoteFailure) { mutable.update { it.copy(error = error.message) } }
    }
    fun enterDemo() {
        try {
            val assets = getApplication<Application>().assets
            val demo = DemoService(assets.open("manifest.json").bufferedReader().use { it.readText() }) {
                path -> assets.open(path).bufferedReader().use { it.readText() }
            }
            reset(true)
            preferences.edit().putBoolean("demo", true).apply()
            observe(demo)
        } catch (_: Exception) { mutable.update { it.copy(error = "演示资源无法读取，请重新安装完整应用。") } }
    }
    fun exitDemo() {
        reset(false)
        configuration = null
        preferences.edit().putBoolean("demo", false).apply()
        observe(remote)
        open(Page(PageKind.CONNECTION, "连接"))
    }
    fun forget() {
        reset(false)
        configuration = null; savedConnection = ""
        preferences.edit().putBoolean("demo", false).apply()
        observe(remote)
        try { credentials.clear() }
        catch (_: Exception) { mutable.update { it.copy(error = "设备未能清除凭据，请在系统设置清除此应用的数据。") } }
        open(Page(PageKind.CONNECTION, "连接"))
    }

    fun open(page: Page) {
        revision++
        mutable.update { it.copy(stack = it.stack + page, detail = JsonNull, error = null,
            snapshot = if (page.route == it.snapshot?.route) it.snapshot else null,
            sessions = if (page.kind == PageKind.SESSIONS) emptyList() else it.sessions) }
        refresh()
    }
    fun back(): Boolean {
        if (state.value.stack.size == 1) return false
        revision++
        mutable.update { it.copy(stack = it.stack.dropLast(1), detail = JsonNull, error = null) }
        refresh()
        return true
    }
    fun setDraft(text: String) { mutable.update { it.copy(drafts = it.drafts + (it.page.route to text)) } }
    fun acknowledge() { mutable.update { it.copy(uncertain = it.uncertain - it.page.route, error = null) } }
    fun changeMonth(delta: Long) {
        revision++
        mutable.update { it.copy(month = it.month.plusMonths(delta), detail = JsonNull) }
        refresh()
    }

    fun refresh() {
        reader?.cancel()
        val start = state.value
        val page = start.page
        val source = service
        val stamp = revision
        val session = source.status.value.session
        if (source.status.value.phase != ConnectionPhase.ONLINE) {
            mutable.update { it.copy(loading = false) }; return
        }
        if (page.kind !in setOf(PageKind.PROJECTS, PageKind.SESSIONS, PageKind.CONVERSATION, PageKind.FILES, PageKind.JOURNEY)) {
            mutable.update { it.copy(loading = false) }; return
        }
        mutable.update { it.copy(loading = true) }
        reader = viewModelScope.launch {
            try {
                val result = when (page.kind) {
                    PageKind.PROJECTS -> source.call("list_projects")
                    PageKind.SESSIONS -> source.call("remote_sessions", page.project)
                    PageKind.CONVERSATION -> source.call("native_conversation_snapshot", page.project, page.route.args)
                    PageKind.FILES -> source.call(if (page.directory) "native_conversation_panel_file_directory" else "native_conversation_panel_file_read",
                        page.project, JsonObject(page.route.args + ("path" to string(page.path))))
                    PageKind.JOURNEY -> {
                        val zone = ZoneId.systemDefault()
                        ResearchHistory.parse(source.call("native_research_journey", page.project, ResearchHistory.monthArgs(start.month, zone)))
                    }
                    else -> JsonNull
                }
                ensureActive()
                if (stamp != revision || source !== service || session != source.status.value.session) return@launch
                when (page.kind) {
                    PageKind.PROJECTS, PageKind.SESSIONS -> {
                        if (result !is JsonArray) throw RemoteFailure(RemoteFailure.Reason.PROTOCOL)
                        mutable.update { if (page.kind == PageKind.PROJECTS) it.copy(projects = result.toList(), error = null)
                            else it.copy(sessions = result.toList(), error = null) }
                    }
                    PageKind.CONVERSATION -> {
                        val snapshot = RemoteSnapshot(result, page.route)
                        mutable.update { if (snapshot.supersedes(it.snapshot?.takeIf { old -> old.route == page.route }))
                            it.copy(snapshot = snapshot, snapshotSession = session, error = null) else it }
                    }
                    else -> mutable.update { it.copy(detail = result, error = null) }
                }
            } catch (error: CancellationException) { throw error }
            catch (error: Exception) {
                if (stamp == revision && source === service) mutable.update {
                    it.copy(error = safeError(error), snapshotSession = if (page.kind == PageKind.CONVERSATION) "" else it.snapshotSession)
                }
            } finally { if (stamp == revision && source === service) mutable.update { it.copy(loading = false) } }
        }
    }

    fun createConversation() {
        val page = state.value.page
        if (state.value.busy || state.value.status.phase != ConnectionPhase.ONLINE || "native_conversation_create" !in state.value.status.commands) return
        val source = service
        val epoch = workspaceEpoch
        mutable.update { it.copy(busy = true) }
        viewModelScope.launch {
            try {
                val id = source.call("native_conversation_create", page.project).text()
                if (id.isBlank()) throw RemoteFailure(RemoteFailure.Reason.PROTOCOL)
                if (epoch == workspaceEpoch && page == state.value.page) open(Page(PageKind.CONVERSATION, "新对话", page.project, id))
            } catch (error: CancellationException) { throw error }
            catch (error: Exception) { if (epoch == workspaceEpoch) mutable.update { it.copy(error = safeError(error)) } }
            finally { if (epoch == workspaceEpoch) mutable.update { it.copy(busy = false) } }
        }
    }

    fun send() {
        val current = state.value
        val text = current.drafts[current.page.route].orEmpty().trim()
        if (text.isEmpty() || current.snapshot?.running == true) return
        mutate("native_conversation_send", obj("message" to string(text), "request_id" to string(UUID.randomUUID().toString()), "attachments" to JsonArray(emptyList())), text)
    }
    fun approve(id: String, approved: Boolean) = mutate("native_conversation_approve", obj(
        "approval_id" to string(id), "approved" to JsonPrimitive(approved), "scope" to string("once")))
    fun stop() = mutate("native_conversation_stop", obj())

    private fun mutate(command: String, extra: JsonObject, sentText: String? = null) {
        val start = state.value
        if (!start.can(command)) return
        val route = start.page.route
        val source = service
        val epoch = workspaceEpoch
        mutable.update { it.copy(busy = true, error = null) }
        viewModelScope.launch {
            try {
                source.call(command, route.project, JsonObject(route.args + extra))
                if (epoch != workspaceEpoch) return@launch
                if (sentText != null) mutable.update {
                    if (it.drafts[route].orEmpty().trim() == sentText) it.copy(drafts = it.drafts - route) else it
                }
                mutable.update { it.copy(snapshotSession = "") }
                refresh()
            } catch (error: CancellationException) { throw error }
            catch (error: Exception) {
                if (epoch == workspaceEpoch) mutable.update { it.copy(error = safeError(error),
                    uncertain = if (error is RemoteFailure && error.reason == RemoteFailure.Reason.UNKNOWN) it.uncertain + route else it.uncertain) }
            } finally { if (epoch == workspaceEpoch) mutable.update { it.copy(busy = false) } }
        }
    }

    fun saveEntry(title: String, text: String) {
        val page = state.value.page
        if (page.kind != PageKind.EDITOR || text.isBlank()) return
        val entry = LocalEntry(UUID.randomUUID().toString(), page.entryKind,
            title.trim().ifEmpty { text.lineSequence().first().take(40) }, text.trim())
        storeEntries(listOf(entry) + state.value.entries, leaveEditor = true)
    }
    fun toggleEntry(entry: LocalEntry) = storeEntries(state.value.entries.map { if (it.id == entry.id) it.copy(done = !it.done) else it })
    private fun storeEntries(entries: List<LocalEntry>, leaveEditor: Boolean = false) {
        if (state.value.localError != null || state.value.localBusy) return
        val page = state.value.page
        mutable.update { it.copy(localBusy = true) }
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) { localStore.save(entries) }
                mutable.update { it.copy(entries = entries) }
                if (leaveEditor && state.value.page == page) back()
            } catch (_: Exception) { mutable.update { it.copy(error = "本机记录保存失败，输入内容已保留，请重试。") } }
            finally { mutable.update { it.copy(localBusy = false) } }
        }
    }
    private fun safeError(error: Exception): String = (error as? RemoteFailure)?.message ?: "读取失败，请重试。"
    override fun onCleared() { service.disconnect(); remote.disconnect(); super.onCleared() }
}
