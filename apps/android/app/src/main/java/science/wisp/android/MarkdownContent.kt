package science.wisp.android

import android.graphics.BitmapFactory
import android.widget.Toast
import androidx.compose.foundation.Image as ComposeImage
import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.*
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.*
import org.commonmark.node.*
import org.commonmark.node.Text as MarkdownText
import org.commonmark.node.Paragraph as MarkdownParagraph
import org.commonmark.ext.gfm.tables.*
import org.commonmark.ext.gfm.strikethrough.Strikethrough
import org.commonmark.ext.task.list.items.TaskListItemMarker
import science.wisp.remote.*

@Composable
internal fun MarkdownContent(text: String, state: WorkspaceState, model: WorkspaceModel,
    resources: List<JsonElement> = emptyList(), directory: String = ".") {
    val document = remember(text) { Markdown.parse(text) }
    val uriHandler = LocalUriHandler.current
    val context = LocalContext.current
    val open: (String) -> Unit = { reference ->
        when (val target = Markdown.target(reference, directory)) {
            is Markdown.Target.ProjectFile -> model.open(Page(PageKind.FILES, target.path.substringAfterLast('/'),
                state.page.project, state.page.session, target.path, directory = false))
            is Markdown.Target.Website -> try { uriHandler.openUri(target.url) }
                catch (_: Exception) { Toast.makeText(context, "没有可打开此链接的浏览器。", Toast.LENGTH_SHORT).show() }
            null -> Unit
        }
    }
    SelectionContainer {
        Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            document.children().forEach { MarkdownBlock(it, state, model, resources, directory, open, depth = 0) }
        }
    }
}

@Composable
private fun MarkdownBlock(node: Node, state: WorkspaceState, model: WorkspaceModel, resources: List<JsonElement>, directory: String, open: (String) -> Unit, depth: Int) {
    // Bound nesting of untrusted content without dropping its readable text.
    if (depth > 24) { Text(plain(node)); return }
    when (node) {
        is Heading -> MarkdownInline(node, open, directory = directory, fontSize = when (node.level) { 1 -> 23; 2 -> 21; else -> 18 }, bold = true)
        is MarkdownParagraph -> {
            val images = descendants(node).filterIsInstance<Image>()
            val onlyImage = node.children().all { it is Image }
            if (!onlyImage) MarkdownInline(node, open, directory = directory)
            images.forEach { image ->
                val label = plain(image).ifBlank { "图片" }
                val source = Markdown.imageSource(image.destination, resources, directory)
                if (source != null) MarkdownImage(state, model, source, label)
                else {
                    MutedText(label)
                    if (Markdown.target(image.destination) is Markdown.Target.Website) TextButton(onClick = { open(image.destination) }) { Text("在浏览器打开图片") }
                }
            }
        }
        is FencedCodeBlock -> CodeBlock(node.literal, node.info.substringBefore(' '))
        is IndentedCodeBlock -> CodeBlock(node.literal, "")
        is BlockQuote -> Row(Modifier.height(IntrinsicSize.Min)) {
            Box(Modifier.width(3.dp).fillMaxHeight().background(MaterialTheme.colorScheme.outlineVariant))
            Column(Modifier.padding(start = 12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                node.children().forEach { MarkdownBlock(it, state, model, resources, directory, open, depth + 1) }
            }
        }
        is BulletList, is OrderedList -> Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
            node.children().forEachIndexed { index, child ->
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    val task = child.children().flatMap { listOf(it) + it.children() }.filterIsInstance<TaskListItemMarker>().firstOrNull()
                    if (task != null) WispIcon(if (task.isChecked) R.drawable.wisp_circle_check else R.drawable.wisp_circle,
                        if (task.isChecked) "已完成" else "未完成", Modifier.size(18.dp))
                    else if (node is OrderedList) Text("${(node.markerStartNumber ?: 1) + index}.", fontSize = 16.sp)
                    else Box(Modifier.padding(top = 9.dp).size(4.dp).background(MaterialTheme.colorScheme.onSurface, RoundedCornerShape(2.dp)))
                    Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                        child.children().forEach { MarkdownBlock(it, state, model, resources, directory, open, depth + 1) }
                    }
                }
            }
        }
        is TableBlock -> MarkdownTable(node, open, directory)
        is ThematicBreak -> RowDivider()
        is HtmlBlock -> CodeBlock(node.literal, "HTML")
        else -> node.children().forEach { MarkdownBlock(it, state, model, resources, directory, open, depth + 1) }
    }
}

@Composable
private fun MarkdownInline(node: Node, open: (String) -> Unit, modifier: Modifier = Modifier, fontSize: Int = 16,
    bold: Boolean = false, align: TextAlign = TextAlign.Start, directory: String = ".") {
    val linkColor = MaterialTheme.colorScheme.primary
    val codeColor = MaterialTheme.colorScheme.background
    val value = remember(node, linkColor, codeColor, open) {
        buildAnnotatedString { appendNodes(node.children(), linkColor, codeColor, open, directory) }
    }
    Text(value, modifier, fontSize = fontSize.sp, lineHeight = (fontSize + 7).sp,
        fontWeight = if (bold) FontWeight.SemiBold else FontWeight.Normal, textAlign = align)
}

private fun AnnotatedString.Builder.appendNodes(nodes: List<Node>, linkColor: Color, codeColor: Color, open: (String) -> Unit, directory: String, depth: Int = 0) {
    for (node in nodes) {
        if (depth > 24) { append(plain(node)); continue }
        when (node) {
            is MarkdownText -> append(node.literal)
            is SoftLineBreak -> append("\n")
            is HardLineBreak -> append("\n")
            is StrongEmphasis -> withStyle(SpanStyle(fontWeight = FontWeight.Bold)) { appendNodes(node.children(), linkColor, codeColor, open, directory, depth + 1) }
            is Emphasis -> withStyle(SpanStyle(fontStyle = FontStyle.Italic)) { appendNodes(node.children(), linkColor, codeColor, open, directory, depth + 1) }
            is Strikethrough -> withStyle(SpanStyle(textDecoration = TextDecoration.LineThrough)) { appendNodes(node.children(), linkColor, codeColor, open, directory, depth + 1) }
            is Code -> withStyle(SpanStyle(fontFamily = FontFamily.Monospace, background = codeColor)) { append(node.literal) }
            is Link -> if (Markdown.target(node.destination, directory) != null) {
                withLink(LinkAnnotation.Clickable(node.destination, TextLinkStyles(SpanStyle(color = linkColor, textDecoration = TextDecoration.Underline)),
                    linkInteractionListener = { open(node.destination) })) { appendNodes(node.children(), linkColor, codeColor, open, directory, depth + 1) }
            } else appendNodes(node.children(), linkColor, codeColor, open, directory, depth + 1)
            is Image -> append(plain(node))
            is HtmlInline -> append(node.literal)
            is TaskListItemMarker -> Unit
            else -> appendNodes(node.children(), linkColor, codeColor, open, directory, depth + 1)
        }
    }
}

@Composable
private fun CodeBlock(code: String, language: String) {
    Surface(shape = RoundedCornerShape(10.dp), color = MaterialTheme.colorScheme.background) {
        Column(Modifier.fillMaxWidth().padding(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            if (language.isNotBlank()) MutedText(language)
            Text(code.trimEnd('\n'), Modifier.horizontalScroll(rememberScrollState()).testTag("markdown-code"),
                fontFamily = FontFamily.Monospace, fontSize = 13.sp, lineHeight = 19.sp, softWrap = false)
        }
    }
}

@Composable
private fun MarkdownTable(node: TableBlock, open: (String) -> Unit, directory: String) {
    val rows = node.children().flatMap { it.children() }.map { it.children().filterIsInstance<TableCell>() }
    // Each column shares one width; long cells wrap and wide tables scroll horizontally.
    val columns = rows.maxOfOrNull { it.size } ?: 0
    val widths = (0 until columns).map { column ->
        (rows.maxOfOrNull { it.getOrNull(column)?.let(::plain)?.length ?: 0 } ?: 0).times(8).plus(24).coerceIn(90, 210).dp
    }
    BoxWithConstraints(Modifier.fillMaxWidth()) {
        val availableWidth = maxWidth
        val tableWidth = widths.fold(0.dp) { total, width -> total + width }
        Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Surface(shape = RoundedCornerShape(10.dp), color = MaterialTheme.colorScheme.background) {
                Column(Modifier.horizontalScroll(rememberScrollState()).width(tableWidth).testTag("markdown-table")) {
                    rows.forEachIndexed { index, cells ->
                        Row(Modifier.height(IntrinsicSize.Min).background(if (index == 0) MaterialTheme.colorScheme.surfaceVariant else Color.Transparent)) {
                            cells.forEachIndexed { column, cell ->
                                MarkdownInline(cell, open, Modifier.width(widths[column]).padding(10.dp), fontSize = 14, bold = cell.isHeader, directory = directory,
                                    align = when (cell.alignment) { TableCell.Alignment.RIGHT -> TextAlign.End; TableCell.Alignment.CENTER -> TextAlign.Center; else -> TextAlign.Start })
                            }
                        }
                        if (index < rows.lastIndex) RowDivider()
                    }
                }
            }
            if (tableWidth > availableWidth) Text("左右滑动查看其余列", color = MaterialTheme.colorScheme.onSurfaceVariant, style = MaterialTheme.typography.bodySmall)
        }
    }
}

@Composable
internal fun MarkdownImage(state: WorkspaceState, model: WorkspaceModel, source: JsonObject, label: String) {
    var retry by remember { mutableIntStateOf(0) }
    var bitmap by remember(source, state.status.session) { mutableStateOf<ImageBitmap?>(null) }
    var error by remember(source, state.status.session) { mutableStateOf<String?>(null) }
    LaunchedEffect(source, state.page.route, state.status.session, state.status.phase, retry) {
        bitmap = null; error = null
        try {
            val bytes = model.image(state.page.route, source)
            bitmap = withContext(Dispatchers.Default) {
                val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
                BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
                if (bounds.outWidth !in 1..8192 || bounds.outHeight !in 1..8192 || bounds.outWidth.toLong() * bounds.outHeight > 16_777_216) throw IllegalArgumentException()
                val options = BitmapFactory.Options().apply {
                    inSampleSize = 1
                    while (bounds.outWidth / inSampleSize > 2048 || bounds.outHeight / inSampleSize > 2048) inSampleSize *= 2
                }
                requireNotNull(BitmapFactory.decodeByteArray(bytes, 0, bytes.size, options)).asImageBitmap()
            }
        } catch (cancelled: CancellationException) { throw cancelled }
        catch (failure: Exception) { error = (failure as? RemoteFailure)?.message ?: "图片无法显示。" }
    }
    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        val preview = bitmap
        if (preview != null) ComposeImage(preview, label, Modifier.fillMaxWidth().aspectRatio(preview.width.toFloat() / preview.height)
            .clip(RoundedCornerShape(10.dp)).testTag("markdown-image"), contentScale = ContentScale.Fit)
        else if (error != null) {
            MutedText(error!!)
            TextButton(onClick = { retry++ }) { Text("重试图片") }
        } else MutedText("读取图片…")
        if (label.isNotBlank()) MutedText(label)
    }
}

private fun descendants(node: Node): List<Node> {
    val result = mutableListOf<Node>()
    val pending = java.util.ArrayDeque<Node>()
    pending.add(node)
    while (pending.isNotEmpty()) {
        val current = pending.removeLast()
        result += current
        current.children().asReversed().forEach(pending::addLast)
    }
    return result
}

private fun plain(node: Node): String = descendants(node).joinToString("") {
    when (it) { is MarkdownText -> it.literal; is Code -> it.literal; is SoftLineBreak, is HardLineBreak -> "\n"; else -> "" }
}
