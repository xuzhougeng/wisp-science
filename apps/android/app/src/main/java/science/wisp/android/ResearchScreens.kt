package science.wisp.android

import androidx.compose.foundation.background
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.util.Locale
import kotlinx.serialization.json.*
import science.wisp.remote.*

@Composable
internal fun ResearchCalendar(state: WorkspaceState, model: WorkspaceModel) {
    val chipColors = FilterChipDefaults.filterChipColors(containerColor = MaterialTheme.colorScheme.background,
        labelColor = MaterialTheme.colorScheme.onSurface, selectedContainerColor = MaterialTheme.colorScheme.primary.copy(alpha = 0.12f),
        selectedLabelColor = MaterialTheme.colorScheme.primary)
    val chipBorder = BorderStroke(0.dp, Color.Transparent)
    val zone = ZoneId.systemDefault()
    val markedDays = state.calendarMonth?.entries.orEmpty().map {
        Instant.ofEpochSecond(it.field("occurred_at").jsonPrimitive.long).atZone(zone).toLocalDate()
    }.toSet()
    LazyColumn(contentPadding = PaddingValues(16.dp), verticalArrangement = Arrangement.spacedBy(22.dp)) {
        item {
            GroupCard {
                Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        FilterChip(modifier = Modifier.testTag("calendar-filter-all"), selected = state.calendarProject == null, onClick = { model.filterCalendar(null) }, label = { Text("所有项目") }, shape = CircleShape, colors = chipColors, border = chipBorder)
                        state.projects.forEach { project ->
                            val id = project.field("id").text()
                            FilterChip(modifier = Modifier.testTag("calendar-filter-$id"), selected = state.calendarProject == id, onClick = { model.filterCalendar(id) },
                                label = { Text(project.field("name").text()) }, shape = CircleShape, colors = chipColors, border = chipBorder)
                        }
                        if (state.calendarProject != null && state.projects.none { it.field("id").text() == state.calendarProject }) {
                            FilterChip(selected = true, onClick = { model.filterCalendar(null) }, label = { Text("不可用项目 · 查看全部") }, shape = CircleShape, colors = chipColors, border = chipBorder)
                        }
                    }
                    RowDivider()
                    Text("${state.month.year}年${state.month.monthValue}月", style = MaterialTheme.typography.titleLarge,
                        fontWeight = FontWeight.Bold, modifier = Modifier.testTag("calendar-month"))
                    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.SpaceBetween) {
                        IconButton(onClick = { model.changeMonth(-1) }) { WispIcon(R.drawable.wisp_chevron_left, "上个月") }
                        TextButton(onClick = { model.selectDay(LocalDate.now()) }) { Text("今天", color = MaterialTheme.colorScheme.onSurface) }
                        IconButton(onClick = { model.changeMonth(1) }) { WispIcon(R.drawable.wisp_chevron_right, "下个月") }
                    }
                    RowDivider()
                    Row(Modifier.fillMaxWidth()) {
                        listOf("一", "二", "三", "四", "五", "六", "日").forEach { day ->
                            Box(Modifier.weight(1f), contentAlignment = Alignment.Center) { MutedText(day) }
                        }
                    }
                    Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
                        ResearchHistory.monthCells(state.month).chunked(7).forEach { week ->
                            Row(Modifier.fillMaxWidth()) {
                                week.forEach { day ->
                                    if (day == null) Spacer(Modifier.weight(1f)) else {
                                        val selected = day == state.selectedDay
                                        val color = if (selected) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurface
                                        Column(Modifier.weight(1f).padding(1.dp).clip(RoundedCornerShape(11.dp))
                                            .background(if (selected) MaterialTheme.colorScheme.primary else Color.Transparent)
                                            .selectable(selected, role = Role.Button, onClick = { model.selectDay(day) })
                                            .testTag("calendar-day-$day")
                                            .semantics { contentDescription = day.format(DateTimeFormatter.ofPattern("yyyy年M月d日")) + if (day in markedDays) "，有研究记录" else "" }
                                            .heightIn(min = 44.dp).padding(vertical = 8.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                                            Text(day.dayOfMonth.toString(), fontSize = 17.sp, color = color)
                                            Box(Modifier.padding(top = 3.dp).size(4.dp).background(if (day in markedDays) {
                                                if (selected) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.primary
                                            } else Color.Transparent, CircleShape))
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        if (state.calendarMonth?.truncated == true) item { MutedText("月份记录较多，部分日期标记可能不完整，请选择日期查看。") }
        item { Text(state.selectedDay.format(DateTimeFormatter.ofPattern("yyyy年M月d日 EEEE", Locale.SIMPLIFIED_CHINESE)),
            Modifier.padding(horizontal = 16.dp), color = MaterialTheme.colorScheme.onSurfaceVariant, fontWeight = FontWeight.SemiBold) }
        item {
            GroupCard {
                Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    val history = state.calendarDay
                    when {
                        state.loading && history == null -> MutedText("正在读取研究记录…")
                        state.error != null -> MutedText("研究记录未能读取，请重试。")
                        history == null -> MutedText("连接主机后可按日期查看研究记录。")
                        else -> {
                            if (history.unavailable.isNotEmpty() || state.calendarMonth?.unavailable?.isNotEmpty() == true) {
                                Text("部分项目不可用或读取失败，当前记录可能不完整。", color = warningColor())
                            }
                            if (history.truncated) MutedText("当天记录较多，未全部返回，请到主机查看完整记录。")
                            if (history.entries.isEmpty() && history.unavailable.isEmpty()) MutedText("当天暂无研究记录。")
                            history.entries.forEach { entry ->
                                Text(entry.field("title").text(), fontWeight = FontWeight.SemiBold)
                                val name = state.projects.firstOrNull { it.field("id") == entry.field("project_id") }?.field("name")?.text().orEmpty()
                                if (name.isNotBlank()) MutedText(name)
                                Text(entry.field("summary").text())
                                if (entry.field("frame_id").text().isNotBlank() && !entry.field("source_discarded").flag()) {
                                    TextButton(onClick = { model.open(Page(PageKind.CONVERSATION, "关联对话", entry.field("project_id").text(), entry.field("frame_id").text())) }) { Text("打开关联对话") }
                                }
                                RowDivider()
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
internal fun Checklist(state: WorkspaceState, model: WorkspaceModel) {
    val entries = state.entries.filter { it.kind == "checklist" }
    val today = LocalDate.now()
    val enabled = !state.localBusy && state.localError == null
    var showCompleted by rememberSaveable { mutableStateOf(false) }
    var draft by rememberSaveable { mutableStateOf("") }
    Column(Modifier.fillMaxSize()) {
        Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 14.dp), horizontalArrangement = Arrangement.SpaceBetween) {
            MutedText("本机清单")
            MutedText("${entries.count { !it.done }} 项待办")
        }
        state.localError?.let { Text(it, Modifier.padding(16.dp), color = MaterialTheme.colorScheme.error) }
        LazyColumn(Modifier.weight(1f), contentPadding = PaddingValues(horizontal = 16.dp)) {
            ChecklistSection.entries.filter { it != ChecklistSection.COMPLETED }.forEach { section ->
                val rows = entries.filter { it.checklistSection(today) == section }.sortedBy { it.dueDate }
                if (rows.isNotEmpty()) {
                    item { Text("${section.label}  ${rows.size}", Modifier.padding(top = 24.dp, bottom = 10.dp), fontSize = 17.sp,
                        fontWeight = FontWeight.SemiBold, color = if (section == ChecklistSection.OVERDUE) warningColor() else MaterialTheme.colorScheme.onSurface) }
                    items(rows, key = { it.id }) { ChecklistRow(it, enabled, model) }
                }
            }
            val completed = entries.filter { it.done }
            if (completed.isNotEmpty()) {
                item { TextButton(onClick = { showCompleted = !showCompleted }, modifier = Modifier.padding(top = 12.dp)) {
                    WispIcon(if (showCompleted) R.drawable.wisp_chevron_down else R.drawable.wisp_chevron_right, null, Modifier.size(18.dp))
                    Text("已完成 · ${completed.size}", Modifier.padding(start = 6.dp), color = MaterialTheme.colorScheme.onSurfaceVariant)
                } }
                if (showCompleted) items(completed, key = { it.id }) { ChecklistRow(it, enabled, model) }
            }
            if (entries.isEmpty() && !state.localBusy) item { MutedText("把待办研究事项记在这里。可添加日期、标记完成，离线也能使用。", Modifier.padding(vertical = 28.dp)) }
        }
        RowDivider()
        Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp), verticalAlignment = Alignment.CenterVertically) {
            Icon(androidx.compose.ui.res.painterResource(R.drawable.wisp_plus), null, tint = MaterialTheme.colorScheme.primary, modifier = Modifier.size(22.dp))
            TextField(draft, { draft = it }, Modifier.weight(1f).testTag("checklist-input"), placeholder = { Text("添加科研事项") }, maxLines = 3,
                colors = TextFieldDefaults.colors(focusedContainerColor = Color.Transparent, unfocusedContainerColor = Color.Transparent,
                    focusedIndicatorColor = Color.Transparent, unfocusedIndicatorColor = Color.Transparent))
            Button(onClick = { model.addChecklist(draft) { draft = "" } }, enabled = enabled && draft.isNotBlank(), shape = RoundedCornerShape(12.dp)) { Text("添加") }
        }
        Text("点圆圈完成，点事项查看或改期 · 暂不与主机同步", Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
            style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

@Composable
private fun ChecklistRow(entry: LocalEntry, enabled: Boolean, model: WorkspaceModel) {
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        IconToggleButton(entry.done, onCheckedChange = { model.toggleEntry(entry) }, enabled = enabled, modifier = Modifier.semantics {
            contentDescription = (if (entry.done) "恢复待办：" else "完成：") + entry.title
        }) {
            Icon(androidx.compose.ui.res.painterResource(if (entry.done) R.drawable.wisp_circle_check else R.drawable.wisp_circle), null,
                tint = MaterialTheme.colorScheme.primary, modifier = Modifier.size(25.dp))
        }
        Column(Modifier.weight(1f).clickable(onClick = { model.editEntry(entry) }).padding(start = 4.dp, top = 16.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text(entry.title, fontSize = 17.sp, textDecoration = if (entry.done) TextDecoration.LineThrough else null,
                color = if (entry.done) MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.onSurface)
            entry.dueDate?.let { date -> if (date != LocalDate.now() || entry.done) Text(date.toString(), style = MaterialTheme.typography.bodySmall,
                color = if (!entry.done && date < LocalDate.now()) warningColor() else MaterialTheme.colorScheme.onSurfaceVariant) }
            RowDivider(Modifier.padding(top = 14.dp))
        }
    }
}

@Composable
internal fun Notebook(state: WorkspaceState, model: WorkspaceModel) {
    LazyColumn(contentPadding = PaddingValues(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        item { Text("你现在想做什么？", Modifier.padding(horizontal = 16.dp, vertical = 8.dp), style = MaterialTheme.typography.titleLarge,
            fontWeight = FontWeight.Bold, color = MaterialTheme.colorScheme.onSurfaceVariant) }
        item { GroupCard {
            NotebookAction(R.drawable.wisp_camera, "拍下纸质实验记录", "拍照与文字识别将在后续版本提供", enabled = false) {}
            RowDivider(Modifier.padding(start = 54.dp, end = 16.dp))
            NotebookAction(R.drawable.wisp_edit, "记下刚才的观察", "记录过程、现象和文字备忘", enabled = !state.localBusy && state.localError == null) {
                model.open(Page(PageKind.EDITOR, "记录观察"))
            }
            RowDivider(Modifier.padding(start = 54.dp, end = 16.dp))
            NotebookAction(R.drawable.wisp_book, "先看一条示例", "了解一份完整记录是什么样") { model.open(Page(PageKind.INFO, "实验记录示例", path = "example")) }
        } }
        item { MutedText("无需连接主机。文字先存在手机；照片和发送到 Wisp 的功能将在后续版本接入。", Modifier.padding(horizontal = 16.dp)) }
        item { Text("本机实验记录", Modifier.padding(horizontal = 16.dp, vertical = 8.dp), fontWeight = FontWeight.SemiBold, color = MaterialTheme.colorScheme.onSurfaceVariant) }
        state.localError?.let { error -> item { Text(error, color = MaterialTheme.colorScheme.error) } }
        val notes = state.entries.filter { it.kind == "notebook" }
        if (notes.isEmpty() && !state.localBusy) item { GroupCard { MutedText("第一条记录会出现在这里。从上方选择写观察，就能开始。", Modifier.padding(16.dp)) } }
        items(notes, key = { it.id }) { entry ->
            GroupCard { Column(Modifier.fillMaxWidth().clickable { model.editEntry(entry) }.padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Text(entry.title, fontWeight = FontWeight.SemiBold)
                Text(entry.text, maxLines = 3, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis, color = MaterialTheme.colorScheme.onSurfaceVariant)
            } }
        }
    }
}

@Composable
private fun NotebookAction(icon: Int, title: String, subtitle: String, enabled: Boolean = true, onClick: () -> Unit) {
    Row(Modifier.fillMaxWidth().clickable(enabled = enabled, onClick = onClick).padding(horizontal = 16.dp, vertical = 24.dp),
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(14.dp)) {
        Icon(androidx.compose.ui.res.painterResource(icon), null, Modifier.size(24.dp), tint = MaterialTheme.colorScheme.primary.copy(alpha = if (enabled) 1f else 0.5f))
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(5.dp)) {
            Text(title, fontSize = 17.sp, fontWeight = FontWeight.SemiBold, color = if (enabled) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant)
            MutedText(subtitle)
        }
        if (enabled) WispChevron()
    }
}

@Composable
internal fun EntryEditor(state: WorkspaceState, model: WorkspaceModel) {
    val old = state.entries.firstOrNull { it.id == state.page.entryId }
    var title by rememberSaveable { mutableStateOf(old?.title.orEmpty()) }
    var text by rememberSaveable { mutableStateOf(old?.text.orEmpty()) }
    var date by rememberSaveable { mutableStateOf(old?.dueDate?.toString().orEmpty()) }
    val checklist = state.page.entryKind == "checklist"
    val parsedDate = runCatching { LocalDate.parse(date) }.getOrNull()
    val invalid = checklist && date.isNotBlank() && parsedDate == null
    Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
        OutlinedTextField(title, { title = it }, Modifier.fillMaxWidth(), label = { Text("标题（可留空）") })
        OutlinedTextField(text, { text = it }, Modifier.fillMaxWidth(), label = { Text("记录内容") }, minLines = if (checklist) 3 else 6)
        if (checklist) {
            OutlinedTextField(date, { date = it }, Modifier.fillMaxWidth(), label = { Text("日期（YYYY-MM-DD，可留空）") }, singleLine = true,
                isError = invalid, supportingText = { if (invalid) Text("请输入有效日期，例如 2026-10-14。") })
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                TextButton(onClick = { date = LocalDate.now().toString() }) { Text("今天") }
                TextButton(onClick = { date = LocalDate.now().plusDays(1).toString() }) { Text("明天") }
                TextButton(onClick = { date = "" }) { Text("不设日期") }
            }
        }
        state.localError?.let { Text(it, color = MaterialTheme.colorScheme.error) }
        Button(onClick = { model.saveEntry(title, text, if (checklist) parsedDate else null) },
            enabled = text.isNotBlank() && !invalid && !state.localBusy && state.localError == null) { Text("保存到本机") }
    }
}

@Composable
internal fun ResearchInfo(kind: String) {
    Column(Modifier.verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
        GroupCard {
            Column(Modifier.padding(20.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
                if (kind == "example") {
                    Text("示例 · 培养物观察", style = MaterialTheme.typography.titleLarge)
                    MutedText("虚构记录，仅用于说明格式；不会写入你的本机记录。")
                    SelectionContainer { Text("目的\n观察不同培养条件下的变化。\n\n过程\n分别标记样本 A 与 B，在相同时间观察并记录。\n\n观察\n样本 A 外观均匀；样本 B 出现少量沉淀。\n\n下一步\n重复观察，并核对温度、时间和操作记录。") }
                } else if (kind == "checklist") {
                    Text("本机科研清单", style = MaterialTheme.typography.titleLarge)
                    Text("添加事项时默认安排在今天。点事项可修改文字和日期，点圆圈可标记完成或恢复待办。")
                    Text("清单按逾期、今天、接下来、未设日期分组。已完成事项可展开查看。日期使用手机本地日期。")
                    MutedText("当前仅保存到手机，不同步主机清单，也不发送提醒。忘记连接会保留记录，卸载应用会删除记录。")
                } else {
                    Text("随手记录研究过程", style = MaterialTheme.typography.titleLarge)
                    Text("无需连接主机即可记录观察。点击已有记录可继续编辑，保存后即保存在本机。")
                    MutedText("拍照、文字识别、附图和发送到 Wisp 尚未接入。应用不会申请相机或相册权限。")
                    MutedText("忘记连接会保留本机记录；清除应用数据或卸载会删除记录。")
                }
            }
        }
    }
}
