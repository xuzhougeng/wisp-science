package science.wisp.android

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.background
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import kotlinx.coroutines.flow.first
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import java.time.format.DateTimeFormatter
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import java.time.Instant
import java.time.ZoneId
import kotlinx.serialization.json.*
import science.wisp.remote.*

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun WorkspaceScreen(model: WorkspaceModel) {
    val state by model.state.collectAsStateWithLifecycle()
    val page = state.page
    val holder = rememberSaveableStateHolder()
    val grouped = page.kind in setOf(PageKind.PROJECTS, PageKind.CALENDAR, PageKind.NOTEBOOK, PageKind.CONNECTION, PageKind.INFO)
    val background = if (grouped) MaterialTheme.colorScheme.background else MaterialTheme.colorScheme.surface
    BackHandler(state.hasBack) { model.back() }
    Scaffold(containerColor = background, topBar = {
        CenterAlignedTopAppBar(colors = TopAppBarDefaults.centerAlignedTopAppBarColors(containerColor = background),
            title = { Text(page.title, fontSize = 18.sp, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis) },
            navigationIcon = {
                if (state.stack.size > 1) ToolbarAction(R.drawable.wisp_chevron_left, "返回", onClick = { model.back() })
                else if (page.kind == PageKind.PROJECTS) ToolbarAction(R.drawable.wisp_gear, "连接设置") {
                    model.open(Page(PageKind.CONNECTION, "连接与演示"))
                }
            }, actions = {
                when (page.kind) {
                    PageKind.PROJECTS -> ToolbarAction(R.drawable.wisp_inbox, "待处理对话",
                        enabled = state.status.phase == ConnectionPhase.ONLINE && "native_conversation_inbox" in state.status.commands) {
                        model.open(Page(PageKind.INBOX, "待处理对话"))
                    }
                    PageKind.SESSIONS -> ToolbarAction(R.drawable.wisp_square_pen, "新建对话", enabled = !state.busy &&
                        state.status.phase == ConnectionPhase.ONLINE && "native_conversation_create" in state.status.commands, onClick = model::createConversation)
                    PageKind.CONVERSATION -> ToolbarAction(R.drawable.wisp_folder, "项目文件") {
                        model.open(Page(PageKind.FILES, "项目文件", page.project, page.session))
                    }
                    PageKind.CHECKLIST -> ToolbarAction(R.drawable.wisp_gear, "清单设置") { model.open(Page(PageKind.INFO, "清单设置", path = "checklist")) }
                    PageKind.NOTEBOOK -> ToolbarAction(R.drawable.wisp_circle_help, "记录本帮助") { model.open(Page(PageKind.INFO, "记录本帮助", path = "notebook")) }
                    PageKind.CALENDAR, PageKind.JOURNEY, PageKind.FILES, PageKind.INBOX -> TextButton(onClick = model::refresh, enabled = !state.loading && state.status.phase == ConnectionPhase.ONLINE) { Text("刷新") }
                    else -> Unit
                }
            })
    }, bottomBar = { if (state.showsTabs) WorkspaceTabs(state, model) }) { padding ->
        Column(Modifier.fillMaxSize().padding(padding).imePadding(), horizontalAlignment = Alignment.CenterHorizontally) {
            Column(Modifier.widthIn(max = 840.dp).fillMaxWidth().weight(1f)) {
                if (state.status.phase != ConnectionPhase.ONLINE && page.kind in setOf(PageKind.SESSIONS, PageKind.CONVERSATION, PageKind.CALENDAR, PageKind.JOURNEY, PageKind.INBOX)) {
                    MutedText(if (state.status.phase == ConnectionPhase.CONNECTING || state.status.phase == ConnectionPhase.RETRYING)
                        "正在连接主机…" else "主机未连接，请在对话页的连接设置中连接。", Modifier.padding(16.dp))
                }
                if (page.kind != PageKind.CONVERSATION) state.error?.let { error ->
                    Text(error, Modifier.padding(16.dp), color = MaterialTheme.colorScheme.error)
                }
                if (state.loading && (page.kind !in setOf(PageKind.PROJECTS, PageKind.SESSIONS, PageKind.CONVERSATION) ||
                    (state.projects.isEmpty() && state.sessions.isEmpty() && state.snapshot == null))) LinearProgressIndicator(Modifier.fillMaxWidth())
                val pageKey = "${state.tab}:$page"
                holder.SaveableStateProvider(pageKey) {
                    DisposableEffect(pageKey) {
                        onDispose {
                            val current = model.state.value
                            val stacks = current.savedStacks + (current.tab to current.stack)
                            if (stacks[state.tab]?.contains(page) != true) holder.removeState(pageKey)
                        }
                    }
                    when (page.kind) {
                        PageKind.PROJECTS -> Projects(state, model)
                        PageKind.CONNECTION -> Connection(state, model)
                        PageKind.SESSIONS -> Sessions(state, model)
                        PageKind.CONVERSATION -> Conversation(state, model)
                        PageKind.FILES -> Files(state, model)
                        PageKind.JOURNEY -> Journey(state, model)
                        PageKind.CALENDAR -> ResearchCalendar(state, model)
                        PageKind.CHECKLIST -> Checklist(state, model)
                        PageKind.NOTEBOOK -> Notebook(state, model)
                        PageKind.EDITOR -> EntryEditor(state, model)
                        PageKind.INFO -> ResearchInfo(state.page.path)
                        PageKind.INBOX -> Inbox(state, model)
                    }
                }
            }
        }
    }
}

@Composable
internal fun NavigationRow(title: String, subtitle: String = "", icon: Int? = null, onClick: () -> Unit) {
    Surface(onClick = onClick, modifier = Modifier.fillMaxWidth(), color = MaterialTheme.colorScheme.surface) {
        Row(Modifier.padding(16.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(14.dp)) {
            icon?.let { androidx.compose.runtime.CompositionLocalProvider(LocalContentColor provides MaterialTheme.colorScheme.primary) { WispIcon(it, null) } }
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Text(title, style = MaterialTheme.typography.titleMedium)
                if (subtitle.isNotEmpty()) Text(subtitle, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            WispChevron()
        }
    }
    HorizontalDivider()
}

@Composable
private fun Projects(state: WorkspaceState, model: WorkspaceModel) {
    LazyColumn(contentPadding = PaddingValues(horizontal = 16.dp, vertical = 16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        item { MutedText(state.status.name.ifBlank { "你的研究工作台" }, Modifier.padding(horizontal = 16.dp, vertical = 4.dp)) }
        item {
            if (state.projects.isNotEmpty()) GroupCard {
                state.projects.forEachIndexed { index, project ->
                    Surface(onClick = { model.open(Page(PageKind.SESSIONS, project.field("name").text(), project.field("id").text())) }) {
                        Row(Modifier.fillMaxWidth().padding(16.dp), verticalAlignment = Alignment.CenterVertically) {
                            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(5.dp)) {
                                Text(project.field("name").text(), fontSize = 18.sp, fontWeight = FontWeight.SemiBold)
                                MutedText("${(project.field("session_count") as? JsonPrimitive)?.intOrNull ?: 0} 个对话")
                                if (((project.field("needs_you_count") as? JsonPrimitive)?.intOrNull ?: 0) > 0) StatusBadge("needs_you")
                                else if (((project.field("running_count") as? JsonPrimitive)?.intOrNull ?: 0) > 0) StatusBadge("running")
                            }
                            WispChevron()
                        }
                    }
                    if (index < state.projects.lastIndex) RowDivider(Modifier.padding(horizontal = 16.dp))
                }
            }
        }
        if (state.projects.isEmpty() && !state.loading) item {
            GroupCard {
                Column(Modifier.padding(20.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text(if (state.status.phase == ConnectionPhase.ONLINE) "暂无可用项目。请在主机创建项目，或检查隐私设置。" else "连接你的 Wisp 主机，或先体验随应用提供的示例项目。")
                    Button(onClick = { model.open(Page(PageKind.CONNECTION, "连接与演示")) }) { Text("连接主机") }
                    OutlinedButton(onClick = model::enterDemo) { Text("体验演示项目") }
                }
            }
        }
    }
}

@Composable
private fun Connection(state: WorkspaceState, model: WorkspaceModel) {
    // Do not put connection secrets into Android's saved-instance-state bundle.
    var address by remember { mutableStateOf("") }
    var code by remember { mutableStateOf("") }
    Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
        Text("连接你的研究工作台", style = MaterialTheme.typography.headlineSmall)
        Text("在电脑端的「设置 → 远程接入 → 网页远程访问」复制链接。计算、模型和项目文件保留在主机上。")
        OutlinedTextField(address, { address = it }, Modifier.fillMaxWidth(), label = { Text("HTTPS 中继地址或完整链接") },
            singleLine = true, visualTransformation = PasswordVisualTransformation())
        OutlinedTextField(code, { code = it }, Modifier.fillMaxWidth(), label = { Text("联机码（完整链接可留空）") },
            singleLine = true, visualTransformation = PasswordVisualTransformation())
        Button(onClick = { model.connect(address, code) }, enabled = address.isNotBlank()) { Text("连接") }
        HorizontalDivider()
        Text("离线演示", style = MaterialTheme.typography.titleMedium)
        Text("浏览 RNA-seq 合成样本、报告和审批流程。演示消息使用本地预设回复，不执行分析。")
        OutlinedButton(onClick = model::enterDemo) { Text(if (state.demo) "重置演示数据" else "体验演示项目") }
        if (state.demo) OutlinedButton(onClick = model::exitDemo) { Text("退出演示") }
        TextButton(onClick = model::forget) { Text("忘记连接并更换服务器") }
        Text("科研清单和记录本保存在本机，忘记连接会保留这些内容。", style = MaterialTheme.typography.bodySmall)
    }
}

@Composable
private fun Sessions(state: WorkspaceState, model: WorkspaceModel) {
    LazyColumn(contentPadding = PaddingValues(horizontal = 16.dp)) {
        item {
            NavigationRow("研究历程", icon = R.drawable.wisp_research_trail) {
                model.open(Page(PageKind.JOURNEY, "研究历程", state.page.project))
            }
        }
        items(state.sessions, key = { it.field("id").text() }) { session -> SessionRow(session) {
            model.open(Page(PageKind.CONVERSATION, session.field("title").text().ifBlank { "对话" }, state.page.project, session.field("id").text()))
        } }
        if (state.sessions.isEmpty() && !state.loading && state.error == null) item { MutedText("暂无对话。", Modifier.padding(16.dp)) }
    }
}

@Composable
private fun Inbox(state: WorkspaceState, model: WorkspaceModel) {
    LazyColumn(contentPadding = PaddingValues(16.dp)) {
        items(state.detail.rows()) { session -> SessionRow(session, showProject = true) {
            model.open(Page(PageKind.CONVERSATION, session.field("title").text(), session.field("project_id").text(), session.field("id").text()))
        } }
        if (state.detail != JsonNull && state.detail.rows().isEmpty() && state.error == null) item { MutedText("暂无待处理对话。") }
    }
}

@Composable
private fun SessionRow(session: JsonElement, showProject: Boolean = false, onClick: () -> Unit) {
    Surface(onClick, Modifier.fillMaxWidth()) {
        Row(Modifier.padding(vertical = 18.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Text(session.field("title").text().ifBlank { "未命名对话" }, fontSize = 17.sp)
                if (showProject) MutedText(session.field("project_name").text())
                StatusBadge(session.field("status").text())
            }
            val stamp = (session.field("activity_at") as? JsonPrimitive)?.longOrNull ?: (session.field("ts") as? JsonPrimitive)?.longOrNull
            stamp?.let { MutedText(Instant.ofEpochSecond(it).atZone(ZoneId.systemDefault()).format(DateTimeFormatter.ofPattern("M月d日"))) }
            WispChevron()
        }
    }
    RowDivider()
}

@Composable
private fun Conversation(state: WorkspaceState, model: WorkspaceModel) {
    val snapshot = state.snapshot?.takeIf { it.route == state.page.route }
    val fresh = snapshot != null && state.snapshotSession == state.status.session && state.status.phase == ConnectionPhase.ONLINE
    var showCompleted by rememberSaveable { mutableStateOf(false) }
    val runs = snapshot?.value?.field("run_cards")?.rows().orEmpty()
    val completed = completedRuns(runs)
    val listState = rememberLazyListState()
    LaunchedEffect(state.page.route, snapshot?.items?.size) {
        if (snapshot != null) {
            val count = snapshotFlow { listState.layoutInfo.totalItemsCount }.first { it > 0 }
            listState.scrollToItem(count - 1)
        }
    }
    Column(Modifier.fillMaxSize()) {
        if (snapshot != null && !fresh) Text("当前内容尚未更新，会话操作已停用。", Modifier.padding(16.dp))
        if (state.page.route in state.uncertain) {
            Text("上次操作结果未确认。核对主机上的对话后，再继续发送或审批。", Modifier.padding(16.dp))
            TextButton(onClick = model::acknowledge) { Text("已核对，允许继续") }
        }
        LazyColumn(Modifier.weight(1f).testTag("transcript"), state = listState, contentPadding = PaddingValues(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            if (snapshot?.value?.field("next_before_seq")?.let { it != JsonNull } == true) item {
                Text("当前显示最近一段对话；完整历史请在主机查看。", style = MaterialTheme.typography.bodySmall)
            }
            itemsIndexed(snapshot?.items.orEmpty()) { index, item ->
                val question = Question.parse(item)
                Surface(color = if (item.field("role").text() == "user") MaterialTheme.colorScheme.secondaryContainer else MaterialTheme.colorScheme.surface,
                    shape = MaterialTheme.shapes.medium) {
                    Column(Modifier.fillMaxWidth().padding(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        Text(roleLabel(item.field("role").text()), style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.primary)
                        if (question != null) {
                            Text(question.text)
                            val answered = snapshot!!.items.drop(index + 1).any { it.field("role").text() == "user" }
                            if (answered) Text("已回答", style = MaterialTheme.typography.labelMedium)
                            else {
                                Text("选择选项会填入输入框，确认后发送。", style = MaterialTheme.typography.bodySmall)
                                question.options.forEach { (label, description) ->
                                    OutlinedButton(onClick = { model.setDraft(listOf(label, description).filter { it.isNotEmpty() }.joinToString("\n")) },
                                        enabled = state.can("native_conversation_send") && !snapshot.running) { Text(label) }
                                }
                            }
                        } else if (item.field("role").text() in setOf("assistant", "user")) {
                            MarkdownContent(item.field("text").text(), state, model, item.field("resources").rows())
                        } else SelectionContainer { Text(item.field("text").text().ifBlank { item.field("tool_name").text() }) }
                    }
                }
            }
            items(snapshot?.value?.field("approvals")?.rows().orEmpty()) { approval ->
                OutlinedCard {
                    Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        Text("需要确认", style = MaterialTheme.typography.titleMedium)
                        Text(approval.field("message").text())
                        SelectionContainer { Text(approval.field("preview").text()) }
                        Button(onClick = { model.approve(approval.field("approval_id").text(), true) }, enabled = state.can("native_conversation_approve")) { Text("批准一次") }
                        OutlinedButton(onClick = { model.approve(approval.field("approval_id").text(), false) }, enabled = state.can("native_conversation_approve")) { Text("拒绝") }
                    }
                }
            }
            items(runs.filter { it.field("status").text() != "succeeded" }, key = { "run-${it.field("id").text()}" }) { RunCard(it) }
            if (completed.isNotEmpty()) {
                item {
                    Surface(onClick = { showCompleted = !showCompleted }, color = MaterialTheme.colorScheme.background, shape = RoundedCornerShape(14.dp)) {
                        Row(Modifier.fillMaxWidth().padding(16.dp), verticalAlignment = Alignment.CenterVertically) {
                            MutedText("已完成的记录（${completed.size}）", Modifier.weight(1f))
                            WispChevron(showCompleted)
                        }
                    }
                }
                if (showCompleted) items(completed, key = { "run-${it.field("id").text()}" }) { RunCard(it) }
            }
            if (snapshot == null && !state.loading) item { Text("连接后刷新，加载当前对话。") }
        }
        RowDivider()
        Column(Modifier.padding(horizontal = 14.dp, vertical = 10.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            state.error?.let { Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall) }
            Surface(color = MaterialTheme.colorScheme.background, shape = RoundedCornerShape(20.dp)) {
                Column(Modifier.padding(8.dp)) {
                    TextField(state.drafts[state.page.route].orEmpty(), model::setDraft,
                        Modifier.fillMaxWidth().semantics { contentDescription = "消息" }, placeholder = { Text("给 Wisp 发消息…") }, minLines = 2, maxLines = 5,
                        colors = TextFieldDefaults.colors(focusedContainerColor = Color.Transparent, unfocusedContainerColor = Color.Transparent,
                            focusedIndicatorColor = Color.Transparent, unfocusedIndicatorColor = Color.Transparent))
                    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically) {
                        TextButton(onClick = {}, enabled = false, modifier = Modifier.weight(1f)) {
                            WispIcon(R.drawable.wisp_image, null, Modifier.size(18.dp)); Spacer(Modifier.width(5.dp)); Text("图片 · 暂未支持", style = MaterialTheme.typography.bodySmall)
                        }
                        if (snapshot?.running == true) TextButton(onClick = model::stop, enabled = state.can("native_conversation_stop") && !snapshot.value.field("stopping").flag()) { Text("停止") }
                        Button(onClick = model::send, shape = RoundedCornerShape(14.dp), enabled = state.can("native_conversation_send") && snapshot?.running == false && state.drafts[state.page.route].orEmpty().isNotBlank()) {
                            Text("发送"); Spacer(Modifier.width(4.dp)); WispIcon(R.drawable.wisp_arrow_up, null, Modifier.size(18.dp))
                        }
                    }
                }
            }
            Text("写文件和执行命令前，会请你批准", Modifier.align(Alignment.CenterHorizontally),
                color = MaterialTheme.colorScheme.onSurfaceVariant, style = MaterialTheme.typography.bodySmall)
        }
    }
}

@Composable
private fun RunCard(run: JsonElement) {
    var expanded by rememberSaveable(run.field("id").text()) { mutableStateOf(false) }
    Surface(onClick = { expanded = !expanded }, color = MaterialTheme.colorScheme.background, shape = RoundedCornerShape(14.dp)) {
        Column(Modifier.fillMaxWidth().padding(14.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(run.field("title").text().ifBlank { "运行记录" }, Modifier.weight(1f), fontSize = 16.sp, fontWeight = FontWeight.Medium)
                StatusBadge(run.field("status").text())
                WispChevron(expanded)
            }
            if (expanded) SelectionContainer {
                Text(listOf("command", "stdout_tail", "stderr_tail", "last_poll_error").map { run.field(it).text() }
                    .filter { it.isNotBlank() }.joinToString("\n\n").ifBlank { "暂无输出" }, style = MaterialTheme.typography.bodySmall)
            }
        }
    }
}

@Composable
private fun Files(state: WorkspaceState, model: WorkspaceModel) {
    if (state.page.directory) LazyColumn {
        items(state.detail.field("entries").rows()) { file ->
            val name = file.field("name").text()
            NavigationRow(name, if (file.field("is_dir").flag()) "文件夹" else if (Markdown.isImage(name)) "图片预览" else "文本预览") {
                val path = if (state.page.path == ".") name else "${state.page.path}/$name"
                model.open(state.page.copy(title = name, path = path, directory = file.field("is_dir").flag()))
            }
        }
        if (state.detail != JsonNull && state.detail.field("entries").rows().isEmpty()) item { Text("空文件夹", Modifier.padding(16.dp)) }
    } else if (Markdown.isImage(state.page.path)) {
        Column(Modifier.verticalScroll(rememberScrollState()).padding(16.dp)) {
            MarkdownImage(state, model, obj("path" to string(state.page.path)), state.page.title)
        }
    } else Column(Modifier.verticalScroll(rememberScrollState()).padding(16.dp)) {
        if (state.detail.field("content").field("truncated").flag()) Text("文件较大，只显示开头部分。")
        val text = state.detail.field("content").field("text").text()
        if (text.isNotBlank() && Markdown.isDocument(state.page.path)) {
            MarkdownContent(text, state, model, directory = state.page.path.substringBeforeLast('/', "."))
        } else SelectionContainer { Text(text.ifBlank {
            if (state.loading) "读取中…" else "暂无文本预览；二进制文件请在主机打开。"
        }) }
    }
}

@Composable
private fun Journey(state: WorkspaceState, model: WorkspaceModel) {
    Column {
        Row(Modifier.fillMaxWidth().padding(8.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.SpaceBetween) {
            TextButton(onClick = { model.changeJourneyMonth(-1) }) { Text("上月") }
            Text(state.journeyMonth.toString(), style = MaterialTheme.typography.titleMedium)
            TextButton(onClick = { model.changeJourneyMonth(1) }) { Text("下月") }
        }
        LazyColumn(contentPadding = PaddingValues(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            if (state.detail.field("truncated").flag()) item { Text("记录较多，当前月份结果未完整返回，请在主机查看。") }
            items(state.detail.field("entries").rows()) { entry ->
                Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    Text(entry.field("title").text(), style = MaterialTheme.typography.titleMedium)
                    val seconds = (entry.field("occurred_at") as? JsonPrimitive)?.longOrNull
                    seconds?.let { Text(Instant.ofEpochSecond(it).atZone(ZoneId.systemDefault()).toLocalDate().toString(), style = MaterialTheme.typography.labelMedium) }
                    Text(entry.field("summary").text())
                    if (entry.field("frame_id").text().isNotBlank() && !entry.field("source_discarded").flag()) TextButton(onClick = {
                        model.open(Page(PageKind.CONVERSATION, "关联对话", state.page.project, entry.field("frame_id").text()))
                    }) { Text("打开关联对话") }
                    HorizontalDivider()
                }
            }
            if (state.detail != JsonNull && state.detail.field("entries").rows().isEmpty() && state.error == null) item { Text("本月暂无研究记录。") }
        }
    }
}

internal fun roleLabel(role: String): String = when (role) {
    "user" -> "你"
    "assistant" -> "Wisp"
    "question" -> "问题"
    "tool" -> "工具"
    "reasoning" -> "思考"
    else -> "记录"
}
