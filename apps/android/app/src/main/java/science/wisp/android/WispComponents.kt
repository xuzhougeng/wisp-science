package science.wisp.android

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

@Composable
internal fun WispIcon(resource: Int, description: String?, modifier: Modifier = Modifier) {
    Icon(painterResource(resource), description, modifier.size(24.dp))
}

@Composable
internal fun ToolbarAction(icon: Int, label: String, enabled: Boolean = true, onClick: () -> Unit) {
    OutlinedIconButton(onClick, Modifier.padding(horizontal = 8.dp).size(46.dp), enabled = enabled,
        shape = CircleShape, colors = IconButtonDefaults.outlinedIconButtonColors(contentColor = MaterialTheme.colorScheme.onSurface), border = BorderStroke(0.5.dp, MaterialTheme.colorScheme.outlineVariant)) {
        WispIcon(icon, label)
    }
}

@Composable
internal fun WorkspaceTabs(state: WorkspaceState, model: WorkspaceModel) {
    Box(Modifier.fillMaxWidth().navigationBarsPadding().padding(horizontal = 18.dp, vertical = 8.dp), contentAlignment = Alignment.Center) {
        Surface(Modifier.widthIn(max = 560.dp).fillMaxWidth(), shape = RoundedCornerShape(40.dp),
            color = MaterialTheme.colorScheme.surface.copy(alpha = 0.97f),
            border = BorderStroke(0.5.dp, MaterialTheme.colorScheme.outlineVariant), shadowElevation = 0.5.dp) {
            Row(Modifier.padding(4.dp)) {
                WorkspaceTab.entries.forEach { tab ->
                    val selected = state.tab == tab
                    val icon = when (tab) {
                        WorkspaceTab.CONVERSATIONS -> R.drawable.wisp_chat
                        WorkspaceTab.CALENDAR -> R.drawable.wisp_calendar
                        WorkspaceTab.CHECKLIST -> R.drawable.wisp_list
                        WorkspaceTab.NOTEBOOK -> R.drawable.wisp_notebook
                    }
                    Column(Modifier.weight(1f).clip(RoundedCornerShape(36.dp))
                        .background(if (selected) MaterialTheme.colorScheme.surfaceVariant else Color.Transparent)
                        .selectable(selected, role = Role.Tab, onClick = { model.selectTab(tab) })
                        .padding(vertical = 9.dp), horizontalAlignment = Alignment.CenterHorizontally,
                        verticalArrangement = Arrangement.spacedBy(3.dp)) {
                        val color = if (selected) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurface
                        Icon(painterResource(icon), null, Modifier.size(25.dp), tint = color)
                        Text(tab.label, fontSize = 11.sp, lineHeight = 14.sp, color = color,
                            fontWeight = if (selected) FontWeight.Medium else FontWeight.Normal)
                    }
                }
            }
        }
    }
}

@Composable
internal fun GroupCard(modifier: Modifier = Modifier, content: @Composable ColumnScope.() -> Unit) {
    Surface(modifier.fillMaxWidth(), shape = RoundedCornerShape(26.dp), color = MaterialTheme.colorScheme.surface) {
        Column(content = content)
    }
}

@Composable
internal fun MutedText(text: String, modifier: Modifier = Modifier) {
    Text(text, modifier, color = MaterialTheme.colorScheme.onSurfaceVariant, style = MaterialTheme.typography.bodyMedium)
}

@Composable
internal fun RowDivider(modifier: Modifier = Modifier) {
    HorizontalDivider(modifier, thickness = 0.5.dp, color = MaterialTheme.colorScheme.outlineVariant)
}

@Composable
internal fun StatusBadge(status: String) {
    if (status.isEmpty() || status == "idle") return
    val color = when (status) {
        "failed", "timed_out", "lost" -> MaterialTheme.colorScheme.error
        "needs_you", "stopping", "cancelling" -> warningColor()
        "running", "succeeded" -> MaterialTheme.colorScheme.primary
        else -> MaterialTheme.colorScheme.onSurfaceVariant
    }
    val label = mapOf("running" to "运行中", "needs_you" to "待处理", "succeeded" to "已完成", "failed" to "失败",
        "timed_out" to "超时", "lost" to "失联", "queued" to "排队中", "submitted" to "排队中", "cancelled" to "已取消",
        "stopping" to "停止中", "cancelling" to "取消中", "paused" to "已暂停", "draft" to "草稿")[status] ?: status
    Surface(color = color.copy(alpha = 0.09f), shape = RoundedCornerShape(6.dp)) {
        Row(Modifier.padding(horizontal = 7.dp, vertical = 4.dp), verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            Box(Modifier.size(5.dp).background(color, CircleShape))
            Text(label, color = color, style = MaterialTheme.typography.labelMedium)
        }
    }
}

@Composable internal fun warningColor(): Color = if (MaterialTheme.colorScheme.surface.red < 0.5f) Color(0xFFE6B86C) else Color(0xFF986315)

@Composable
internal fun WispChevron(expanded: Boolean = false) {
    Icon(painterResource(if (expanded) R.drawable.wisp_chevron_down else R.drawable.wisp_chevron_right), null,
        Modifier.size(18.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.6f))
}
