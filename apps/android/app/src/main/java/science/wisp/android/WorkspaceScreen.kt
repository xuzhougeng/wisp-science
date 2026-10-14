package science.wisp.android

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
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
    BackHandler(state.stack.size > 1) { model.back() }
    Scaffold(topBar = {
        TopAppBar(title = { Text(page.title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
            navigationIcon = {
                if (state.stack.size > 1) IconButton(onClick = { model.back() }) { WispIcon(R.drawable.wisp_chevron_left, "返回") }
            }, actions = {
                if (page.kind == PageKind.PROJECTS) IconButton(onClick = { model.open(Page(PageKind.CONNECTION, "连接与演示")) }) {
                    WispIcon(R.drawable.wisp_gear, "连接设置")
                }
                if (page.kind == PageKind.CONVERSATION) IconButton(onClick = {
                    model.open(Page(PageKind.FILES, "项目文件", page.project, page.session))
                }) { WispIcon(R.drawable.wisp_folder, "项目文件") }
                if (page.kind in setOf(PageKind.PROJECTS, PageKind.SESSIONS, PageKind.CONVERSATION, PageKind.FILES, PageKind.JOURNEY)) {
                    TextButton(onClick = model::refresh, enabled = !state.loading && state.status.phase == ConnectionPhase.ONLINE) { Text("刷新") }
                }
            })
    }) { padding ->
        Column(Modifier.fillMaxSize().padding(padding).imePadding(), horizontalAlignment = Alignment.CenterHorizontally) {
            Column(Modifier.widthIn(max = 840.dp).fillMaxWidth().weight(1f)) {
                val connection = when {
                    state.demo -> "离线演示 · 合成数据"
                    state.status.phase == ConnectionPhase.ONLINE -> "已连接 · ${state.status.name}"
                    state.status.phase == ConnectionPhase.CONNECTING -> "正在连接主机…"
                    state.status.phase == ConnectionPhase.RETRYING -> "连接中断，正在重连；会话操作已停用"
                    state.status.phase == ConnectionPhase.OFFLINE -> "主机离线，请保持主机联网并运行 Wisp"
                    else -> "未连接 · 本机科研记录仍可使用"
                }
                Text(connection, Modifier.padding(horizontal = 16.dp, vertical = 8.dp), style = MaterialTheme.typography.labelLarge)
                state.error?.let {
                    Text(it, Modifier.padding(horizontal = 16.dp, vertical = 8.dp), color = MaterialTheme.colorScheme.error)
                }
                if (state.loading) LinearProgressIndicator(Modifier.fillMaxWidth())
                key(page) {
                    when (page.kind) {
                        PageKind.PROJECTS -> Projects(state, model)
                        PageKind.CONNECTION -> Connection(state, model)
                        PageKind.SESSIONS -> Sessions(state, model)
                        PageKind.CONVERSATION -> Conversation(state, model)
                        PageKind.FILES -> Files(state, model)
                        PageKind.JOURNEY -> Journey(state, model)
                        PageKind.CHECKLIST, PageKind.NOTEBOOK -> LocalResearch(state, model)
                        PageKind.EDITOR -> EntryEditor(state, model)
                    }
                }
            }
        }
    }
}

@Composable
private fun WispIcon(resource: Int, description: String?) {
    Icon(painterResource(resource), description, Modifier.size(24.dp))
}

@Composable
private fun NavigationRow(title: String, subtitle: String = "", icon: Int? = null, onClick: () -> Unit) {
    Surface(onClick = onClick, modifier = Modifier.fillMaxWidth(), color = MaterialTheme.colorScheme.surface) {
        Row(Modifier.padding(16.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(14.dp)) {
            icon?.let { WispIcon(it, null) }
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Text(title, style = MaterialTheme.typography.titleMedium)
                if (subtitle.isNotEmpty()) Text(subtitle, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            WispIcon(R.drawable.wisp_chevron_right, null)
        }
    }
    HorizontalDivider()
}

@Composable
private fun Projects(state: WorkspaceState, model: WorkspaceModel) {
    LazyColumn {
        item { NavigationRow("科研清单", "本机保存 · 离线可用", R.drawable.wisp_list) { model.open(Page(PageKind.CHECKLIST, "科研清单")) } }
        item { NavigationRow("实验记录本", "文字记录 · 离线可用", R.drawable.wisp_notebook) { model.open(Page(PageKind.NOTEBOOK, "实验记录本")) } }
        item { Text("项目", Modifier.padding(16.dp), style = MaterialTheme.typography.titleLarge) }
        items(state.projects) { project ->
            NavigationRow(project.field("name").text(), project.field("description").text(), R.drawable.wisp_folder) {
                model.open(Page(PageKind.SESSIONS, project.field("name").text(), project.field("id").text()))
            }
        }
        if (state.projects.isEmpty() && !state.loading) item {
            Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text(if (state.status.phase == ConnectionPhase.ONLINE) "暂无可用项目。请在主机创建项目，或检查隐私设置。" else "连接你的 Wisp 主机，或先体验随应用提供的示例项目。")
                Button(onClick = { model.open(Page(PageKind.CONNECTION, "连接与演示")) }) { Text("连接主机") }
                OutlinedButton(onClick = model::enterDemo) { Text("体验演示项目") }
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
    LazyColumn {
        item {
            NavigationRow("研究历程", "按月阅读项目研究记录", R.drawable.wisp_research_trail) {
                model.open(Page(PageKind.JOURNEY, "研究历程", state.page.project))
            }
        }
        item {
            TextButton(onClick = model::createConversation, modifier = Modifier.padding(8.dp), enabled = !state.busy &&
                state.status.phase == ConnectionPhase.ONLINE && "native_conversation_create" in state.status.commands) {
                WispIcon(R.drawable.wisp_square_pen, null); Spacer(Modifier.width(8.dp)); Text("新建对话")
            }
        }
        items(state.sessions) { session ->
            NavigationRow(session.field("title").text().ifBlank { "未命名对话" }, session.field("status").text()) {
                model.open(Page(PageKind.CONVERSATION, session.field("title").text().ifBlank { "对话" }, state.page.project, session.field("id").text()))
            }
        }
        if (state.sessions.isEmpty() && !state.loading && state.error == null) item { Text("暂无对话。", Modifier.padding(16.dp)) }
    }
}

@Composable
private fun Conversation(state: WorkspaceState, model: WorkspaceModel) {
    val snapshot = state.snapshot?.takeIf { it.route == state.page.route }
    val fresh = snapshot != null && state.snapshotSession == state.status.session && state.status.phase == ConnectionPhase.ONLINE
    Column(Modifier.fillMaxSize()) {
        if (snapshot != null && !fresh) Text("当前内容尚未更新，会话操作已停用。", Modifier.padding(16.dp))
        if (state.page.route in state.uncertain) {
            Text("上次操作结果未确认。核对主机上的对话后，再继续发送或审批。", Modifier.padding(16.dp))
            TextButton(onClick = model::acknowledge) { Text("已核对，允许继续") }
        }
        LazyColumn(Modifier.weight(1f).testTag("transcript"), contentPadding = PaddingValues(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
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
            items(snapshot?.value?.field("run_cards")?.rows().orEmpty()) { run ->
                var expanded by rememberSaveable(run.field("id").text()) { mutableStateOf(false) }
                OutlinedCard(onClick = { expanded = !expanded }) {
                    Column(Modifier.fillMaxWidth().padding(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        Text(run.field("title").text().ifBlank { "运行记录" }, style = MaterialTheme.typography.titleSmall)
                        Text(run.field("status").text())
                        if (expanded) SelectionContainer {
                            Text(listOf("command", "stdout_tail", "stderr_tail").map { run.field(it).text() }.filter { it.isNotBlank() }.joinToString("\n"))
                        }
                    }
                }
            }
            if (snapshot == null && !state.loading) item { Text("连接后刷新，加载当前对话。") }
        }
        HorizontalDivider()
        Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedTextField(state.drafts[state.page.route].orEmpty(), model::setDraft, Modifier.fillMaxWidth(),
                label = { Text("消息") }, maxLines = 5)
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End, verticalAlignment = Alignment.CenterVertically) {
                if (snapshot?.running == true) TextButton(onClick = model::stop, enabled = state.can("native_conversation_stop") && !snapshot.value.field("stopping").flag()) { Text("停止") }
                Button(onClick = model::send, enabled = state.can("native_conversation_send") && snapshot?.running == false && state.drafts[state.page.route].orEmpty().isNotBlank()) {
                    WispIcon(R.drawable.wisp_arrow_up, null); Spacer(Modifier.width(6.dp)); Text("发送")
                }
            }
        }
    }
}

@Composable
private fun Files(state: WorkspaceState, model: WorkspaceModel) {
    if (state.page.directory) LazyColumn {
        items(state.detail.field("entries").rows()) { file ->
            val name = file.field("name").text()
            NavigationRow(name, if (file.field("is_dir").flag()) "文件夹" else "文本预览") {
                val path = if (state.page.path == ".") name else "${state.page.path}/$name"
                model.open(state.page.copy(title = name, path = path, directory = file.field("is_dir").flag()))
            }
        }
        if (state.detail != JsonNull && state.detail.field("entries").rows().isEmpty()) item { Text("空文件夹", Modifier.padding(16.dp)) }
    } else Column(Modifier.verticalScroll(rememberScrollState()).padding(16.dp)) {
        if (state.detail.field("content").field("truncated").flag()) Text("文件较大，只显示开头部分。")
        SelectionContainer { Text(state.detail.field("content").field("text").text().ifBlank {
            if (state.loading) "读取中…" else "暂无文本预览；图片和二进制文件请在主机打开。"
        }) }
    }
}

@Composable
private fun Journey(state: WorkspaceState, model: WorkspaceModel) {
    Column {
        Row(Modifier.fillMaxWidth().padding(8.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.SpaceBetween) {
            TextButton(onClick = { model.changeMonth(-1) }) { Text("上月") }
            Text(state.month.toString(), style = MaterialTheme.typography.titleMedium)
            TextButton(onClick = { model.changeMonth(1) }) { Text("下月") }
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

@Composable
private fun LocalResearch(state: WorkspaceState, model: WorkspaceModel) {
    val kind = if (state.page.kind == PageKind.CHECKLIST) "checklist" else "notebook"
    LazyColumn(contentPadding = PaddingValues(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        item {
            Text(if (kind == "checklist") "本机清单暂不与主机同步。" else "记录保存在本机；拍照、文字识别和发送到主机将在后续版本接入。")
            state.localError?.let { Text(it, color = MaterialTheme.colorScheme.error) }
            Button(onClick = { model.open(Page(PageKind.EDITOR, "新增记录", entryKind = kind)) }, enabled = state.localError == null && !state.localBusy) { Text("新增记录") }
        }
        items(state.entries.filter { it.kind == kind }, key = { it.id }) { entry ->
            OutlinedCard {
                Row(Modifier.fillMaxWidth().padding(12.dp), verticalAlignment = Alignment.Top) {
                    if (kind == "checklist") Checkbox(entry.done, { model.toggleEntry(entry) }, enabled = !state.localBusy && state.localError == null)
                    Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                        Text(entry.title, style = MaterialTheme.typography.titleMedium)
                        SelectionContainer { Text(entry.text) }
                    }
                }
            }
        }
        if (state.entries.none { it.kind == kind } && !state.localBusy) item { Text("暂无本机记录。") }
    }
}

@Composable
private fun EntryEditor(state: WorkspaceState, model: WorkspaceModel) {
    var title by rememberSaveable { mutableStateOf("") }
    var text by rememberSaveable { mutableStateOf("") }
    Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
        OutlinedTextField(title, { title = it }, Modifier.fillMaxWidth(), label = { Text("标题（可留空）") })
        OutlinedTextField(text, { text = it }, Modifier.fillMaxWidth(), label = { Text("记录内容") }, minLines = 6)
        Button(onClick = { model.saveEntry(title, text) }, enabled = text.isNotBlank() && !state.localBusy && state.localError == null) { Text("保存到本机") }
    }
}

private fun roleLabel(role: String): String = when (role) {
    "user" -> "你"
    "assistant" -> "Wisp"
    "question" -> "问题"
    "tool" -> "工具"
    "thinking" -> "思考"
    else -> "记录"
}
