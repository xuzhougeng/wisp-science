using System.Text.Json;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Shapes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class NativeJourneyPage
{
    private Grid? dialog;
    private Border? dialogCard;
    private string? dialogKey;
    private Control? previousFocus;
    private NativeRichPreview? preview;
    private bool artifactOpen;
    private CalendarDatePicker? noteDate;
    private ComboBox? noteCategory;
    private TextBox? noteTitle, noteBody;
    private TextBlock? noteError, recapError;
    private Button? saveNote, saveRecap;
    private JourneyRecap? recapEditor;
    private JourneyGraphEdge? selectedEdge;
    private JourneyGraph? renderedGraph;
    private bool renderedGraphCanvas, renderedGraphBusy;
    private string? renderedGraphError;

    private void RenderDetails()
    {
        details.Children.Clear();
        details.Children.Add(Text(model.Selected is { Kind: not "artifact" } ? L("记录详情", "Record details") : L("产物来源", "Source details"), 16, bold: true));
        if (model.Selected is not { } entry) { details.Children.Add(Text(L("选择一份产出或研究记录，查看它的来源。", "Select an output or record to trace its source."), 12, "text-faint")); return; }
        details.Children.Add(Text(entry.Title, 14, bold: true));
        details.Children.Add(Text(DateTimeOffset.FromUnixTimeSeconds(entry.OccurredAt).LocalDateTime.ToString("yyyy-MM-dd HH:mm") + " · " + (entry.Manual ? L("手动记录", "Manually recorded") : L("已记录", "Recorded")), 12, "text-faint"));
        if (entry.Summary.Length > 0) { var text = Text(entry.Summary, 12); text.IsTextSelectionEnabled = true; details.Children.Add(text); }
        if (entry.Manual && entry.RecordedAt is { } recorded) details.Children.Add(Text(L("补记于 ", "Added ") + DateTimeOffset.FromUnixTimeSeconds(recorded).LocalDateTime.ToString("g"), 11, "text-faint"));
        if (entry.SourceDiscarded) details.Children.Add(Text(L("源文件已不可用；保留的记录信息仍可查看。", "The source file is unavailable. Recorded information is still available."), 12, "text-faint"));
        if (model.SourceLoading) details.Children.Add(Text(L("正在读取来源…", "Loading source…"), 12, "text-faint"));
        if (model.SourceError != null) { details.Children.Add(Text(model.SourceError, 12, "err")); if (entry.Kind == "artifact" && entry.SourceId is { } sourceId) details.Children.Add(Link(L("重试", "Try again"), () => _ = model.OpenArtifactAsync(sourceId))); }
        if (openSession != null && !string.IsNullOrEmpty(entry.FrameId)) details.Children.Add(Link(L("查看来源会话", "Open source conversation"), () => openSession(entry.FrameId), 12));
        if (model.SourceRunId != null) details.Children.Add(Link(L("查看来源运行", "Open producing run"), () => _ = model.OpenRunAsync()));
        if (model.Artifact is not { } artifact) return;
        if (model.HasParentSource) details.Children.Add(Link(L("返回上级来源", "Back to parent source"), model.Back));
        details.Children.Add(Text(artifact.Filename + $" · v{artifact.VersionNumber}", 13, bold: true));
        details.Children.Add(Text(artifact.Source.RunTitle.Length > 0 ? artifact.Source.RunTitle + " · " + artifact.Source.RunStatus : L("尚未记录生成它的运行", "No producing run has been recorded."), 12, "text-faint"));
        if (artifact.Source.ContextId.Length > 0) details.Children.Add(Text(L("执行环境 · ", "Execution context · ") + artifact.Source.ContextId, 11, "text-faint"));
        details.Children.Add(Text(L("输入来源", "Inputs"), 12, bold: true));
        if (artifact.Source.Inputs.Length == 0) details.Children.Add(Text(L("尚未记录输入数据", "No input data has been recorded."), 12, "text-faint"));
        foreach (var input in artifact.Source.Inputs)
        {
            if (input.VersionId is { } id) details.Children.Add(Link(input.Title, () => _ = model.OpenArtifactAsync(id)));
            else details.Children.Add(Text(input.Title, 12));
            details.Children.Add(Text(input.Role + " · " + input.Confidence, 11, "text-faint"));
        }
        details.Children.Add(Action(L("打开产出", "Open output"), () => { artifactOpen = true; RenderDialog(); }));
    }
    private static (string En, string Zh, JourneyRecapItem[] Items)[] Sections(JourneyRecap r) =>
        [("Done", "今日完成", r.Done), ("Findings & decisions", "发现与决定", r.Findings), ("Issues", "问题与失败", r.Issues), ("Next", "接下来", r.Next)];
    private FrameworkElement RecapCard(JourneyRecap recap)
    {
        var content = new StackPanel { Spacing = 12 };
        recapError = Text(model.SaveError ?? "", 12, "err");
        if (recapEditor?.Id == recap.Id)
        {
            var title = Editor(recapEditor.Headline, false); content.Children.Add(Text(L("标题", "Headline"), 12)); content.Children.Add(title);
            var boxes = new List<TextBox>();
            foreach (var section in Sections(recapEditor)) { content.Children.Add(Text(L(section.Zh, section.En), 12)); var box = Editor(string.Join("\n", section.Items.Select(i => i.Text)), true); box.Height = 88; boxes.Add(box); content.Children.Add(box); }
            content.Children.Add(Text(L("每行一条；改动过的行不再关联原来源。", "One item per line. A line you change no longer cites its sources."), 11, "text-faint")); content.Children.Add(recapError);
            var actions = new NativeActionWrap { Gap = 8, RightAligned = true }; actions.Children.Add(Action(L("取消", "Cancel"), () => { recapEditor = null; rendered = null; RenderFeed(); }));
            saveRecap = Action(L("保存并确认", "Save and confirm"), async () =>
            {
                var parts = boxes.Select(b => NativeJourneyPresentation.RecapItems(b.Text)).ToArray();
                if (await model.SaveRecapAsync(new(recap.Id, "confirmed", title.Text, parts[0], parts[1], parts[2], parts[3]))) { recapEditor = null; rendered = null; RenderFeed(); }
            }, "save", true); actions.Children.Add(saveRecap); content.Children.Add(actions);
        }
        else
        {
            var head = new NativeActionWrap { Gap = 8 }; head.Children.Add(design.Icon("sparkles", 16, design.ColorText("clay-strong"))); head.Children.Add(Text(L("每日回顾", "Daily recap"), 13, bold: true)); head.Children.Add(Text(recap.Status == "confirmed" ? L("已确认", "Confirmed") : L("草稿", "Draft"), 11, "text-faint")); content.Children.Add(head);
            var actions = new NativeActionWrap { Gap = 6 };
            if (recap.Status != "confirmed") actions.Children.Add(Link(L("确认", "Confirm"), () => _ = model.SaveRecapAsync(new(recap.Id, "confirmed", recap.Headline, recap.Done, recap.Findings, recap.Issues, recap.Next))));
            actions.Children.Add(Link(L("编辑", "Edit"), () => { recapEditor = recap; rendered = null; RenderFeed(); }));
            actions.Children.Add(Icon(L("忽略回顾", "Dismiss recap"), "eye-off", () => _ = model.SaveRecapAsync(new(recap.Id, "dismissed", recap.Headline, recap.Done, recap.Findings, recap.Issues, recap.Next)))); content.Children.Add(actions);
            foreach (var section in Sections(recap).Where(s => s.Items.Length > 0))
            {
                content.Children.Add(Text(L(section.Zh, section.En), 12, bold: true));
                foreach (var item in section.Items)
                {
                    content.Children.Add(Text("• " + item.Text, 12, "text-muted"));
                    foreach (var index in item.Refs.Where(i => i >= 0 && i < recap.Sources.Length))
                    {
                        var source = recap.Sources[index]; content.Children.Add(Link(source.Title, async () =>
                        {
                            if (source.Kind == "session" && openSession != null) { openSession(source.Id); return; }
                            var entry = model.Page?.Entries.FirstOrDefault(e => e.SourceId == source.Id || e.Id == source.Id);
                            if (entry != null) { await model.SelectAsync(entry); if (source.Kind == "run") await model.OpenRunAsync(); }
                        }, 11));
                    }
                }
            }
            content.Children.Add(Text(L($"由 {recap.Model} 起草，可在 设置 → 专家 → Recap 中更换模型。", $"Drafted by {recap.Model}. Change the model in Settings → Specialists → Recap."), 11, "text-faint")); content.Children.Add(recapError);
        }
        return new Border { Child = content, Padding = new(16, 14, 16, 14), Background = design.Brush("bg-elev"), BorderBrush = design.Brush("border"), BorderThickness = new(1), CornerRadius = new(8) };
    }
    private TextBox Editor(string value, bool multiline)
    {
        var box = new TextBox { AcceptsReturn = multiline, Text = value, TextWrapping = multiline ? TextWrapping.Wrap : TextWrapping.NoWrap, MinWidth = 0, Padding = new(12, 8, 12, 8), Background = design.Brush("bg-input"), Foreground = design.Brush("text"), BorderBrush = design.Brush("border"), BorderThickness = new(1), CornerRadius = new(8) };
        design.BindTypography(box, size: 14); box.Resources["TextControlBackgroundFocused"] = design.Brush("bg-input"); box.Resources["TextControlBorderBrushFocused"] = design.Brush("clay"); TextMenu(box); return box;
    }
    private void RenderDialog()
    {
        var key = model.NoteOpen ? "note" : model.Run != null ? "run:" + model.Run.Id : artifactOpen && model.Artifact != null ? "artifact:" + model.Artifact.VersionId : null;
        if (key != dialogKey)
        {
            RemoveDialog(); dialogKey = key;
            if (key == null) { presentationChanged?.Invoke(); return; }
            previousFocus = FocusManager.GetFocusedElement(XamlRoot) as Control;
            dialog = new Grid(); NativeDialogVisuals.Backdrop(dialog, design);
            dialog.Tapped += (_, e) => { if (ReferenceEquals(e.OriginalSource, dialog)) HandleEscape(); };
            var content = new StackPanel { Spacing = 20, Padding = new(24), TabFocusNavigation = KeyboardNavigationMode.Cycle };
            var title = model.NoteOpen ? L("补充研究记录", "Add research entry") : model.Run != null ? L("运行记录", "Run record") : model.Artifact!.Filename;
            var head = new Grid(); Columns(head, new(1, GridUnitType.Star), GridLength.Auto); head.Children.Add(Serif(title, 23)); Put(head, Icon(L("关闭", "Close"), "close", HandleEscape), 0, 1); content.Children.Add(head);
            if (model.NoteOpen) NoteForm(content); else if (model.Run != null) RunDetail(content, model.Run); else ArtifactDetail(content, model.Artifact!);
            dialogCard = new Border { Background = design.Brush("bg-elev"), BorderBrush = design.Brush("border"), BorderThickness = new(1), CornerRadius = new(16), Child = Scroll(content), HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center };
            dialog.Children.Add(dialogCard); layers.Children.Add(dialog); pageControl.IsEnabled = false; ResizeDialog();
            presentationChanged?.Invoke();
            DispatcherQueue.TryEnqueue(() => { if (dialog != null) (noteTitle ?? FocusManager.FindFirstFocusableElement(dialog) as Control)?.Focus(FocusState.Programmatic); });
        }
        if (saveNote != null) saveNote.IsEnabled = model.CanSaveNote;
        if (saveRecap != null) saveRecap.IsEnabled = !model.Saving;
        if (noteError != null) noteError.Text = model.SaveError ?? "";
        if (recapError != null) recapError.Text = model.SaveError ?? "";
        if (noteTitle != null) noteTitle.IsReadOnly = model.Saving;
        if (noteBody != null) noteBody.IsReadOnly = model.Saving;
        if (noteDate != null) noteDate.IsEnabled = !model.Saving;
        if (noteCategory != null) noteCategory.IsEnabled = !model.Saving;
    }
    private void ResizeDialog()
    { if (dialogCard != null) { dialogCard.Width = Math.Max(0, Math.Min(model.NoteOpen ? 480 : 900, ActualWidth - 40)); dialogCard.MaxHeight = Math.Max(0, ActualHeight - 40); } }
    private void RemoveDialog()
    {
        preview?.Dispose(); preview = null;
        if (dialog != null) layers.Children.Remove(dialog);
        dialog = null; dialogCard = null; dialogKey = null; noteDate = null; noteCategory = null; noteTitle = noteBody = null; noteError = null; saveNote = null; pageControl.IsEnabled = true;
        if (previousFocus is { IsLoaded: true }) previousFocus.Focus(FocusState.Programmatic); previousFocus = null;
    }
    private void NoteForm(StackPanel content)
    {
        content.Children.Add(Text(L("记录进展、发现、决策或下一步。研究日期与补记时间会分别保存。", "Record progress, a finding, a decision or a next step. The entry date and the time you added it are kept separately."), 12, "text-faint"));
        var row = new Grid { ColumnSpacing = 16 }; Columns(row, new(1, GridUnitType.Star), new(1, GridUnitType.Star));
        noteDate = new CalendarDatePicker { Header = L("研究日期", "Research date"), Date = new DateTimeOffset(model.NoteDate), HorizontalAlignment = HorizontalAlignment.Stretch, MinWidth = 0 }; noteDate.DateChanged += (_, _) => { if (noteDate?.Date is { } date) model.NoteDate = date.Date; };
        noteCategory = new ComboBox { Header = L("记录类型", "Category"), HorizontalAlignment = HorizontalAlignment.Stretch, MinWidth = 0 };
        foreach (var kind in new[] { "progress", "finding", "decision", "next" }) noteCategory.Items.Add(new ComboBoxItem { Tag = kind, Content = Kind(kind) });
        noteCategory.SelectedIndex = 0; noteCategory.SelectionChanged += (_, _) => { if (noteCategory?.SelectedItem is ComboBoxItem item) model.NoteCategory = (string)item.Tag; };
        row.Children.Add(noteDate); Put(row, noteCategory, 0, 1); content.Children.Add(row);
        noteTitle = Editor(model.NoteTitle, false); noteTitle.Header = L("标题", "Title"); noteTitle.MaxLength = 200;
        noteTitle.TextChanged += (_, _) => { if (noteTitle == null) return; model.NoteTitle = noteTitle.Text; if (saveNote != null) saveNote.IsEnabled = model.CanSaveNote; }; content.Children.Add(noteTitle);
        noteBody = Editor(model.NoteBody, true); noteBody.Header = L("详情与依据", "Details and evidence"); noteBody.Height = 150; noteBody.MaxLength = 10000;
        noteBody.TextChanged += (_, _) => { if (noteBody != null) model.NoteBody = noteBody.Text.ReplaceLineEndings("\n"); }; content.Children.Add(noteBody);
        noteError = Text("", 12, "err"); content.Children.Add(noteError);
        var actions = new NativeActionWrap { Gap = 10, RightAligned = true }; actions.Children.Add(Action(L("取消", "Cancel"), model.CloseNote)); saveNote = Action(L("保存记录", "Save entry"), () => _ = model.SaveNoteAsync(), primary: true); actions.Children.Add(saveNote); content.Children.Add(actions);
    }
    private void RunDetail(StackPanel content, NativeRun run)
    {
        content.Children.Add(Text(run.Title, 18, bold: true)); content.Children.Add(Text(run.Status + " · " + run.ContextId, 12, "text-faint"));
        if (run.ExitCode != null) content.Children.Add(Text(L("退出码：", "Exit code: ") + run.ExitCode, 12));
        foreach (var field in new[] { (L("命令", "Command"), run.Command), (L("工作目录", "Working directory"), run.RemoteWorkdir), (L("标准输出（尾部）", "Standard output (tail)"), run.StdoutTail), (L("标准错误（尾部）", "Standard error (tail)"), run.StderrTail), (L("最近轮询错误", "Last poll error"), run.LastPollError) })
        { if (string.IsNullOrEmpty(field.Item2)) continue; content.Children.Add(Text(field.Item1, 12, bold: true)); var box = Editor(field.Item2, true); box.IsReadOnly = true; box.MaxHeight = 280; content.Children.Add(box); }
    }
    private void ArtifactDetail(StackPanel content, JourneyArtifact artifact)
    {
        if (artifact.ContentError != null) { content.Children.Add(Text(artifact.ContentError, 13, "err")); return; }
        if (artifact.Base64 != null && artifact.Mime.StartsWith("image/", StringComparison.Ordinal))
        {
            if (artifact.Mime == "image/svg+xml")
            { preview = new NativeRichPreview(design, "image", artifact.Base64, artifact.Filename, escape: HandleEscape, format: artifact.Mime) { MaxHeight = Math.Max(120, ActualHeight * .65) }; content.Children.Add(preview); }
            else
            {
                var host = new Border { MinHeight = 80, MaxHeight = Math.Max(120, ActualHeight * .65) }; host.Child = Text(L("正在加载图像…", "Loading image…"), 13, "text-faint"); content.Children.Add(host); _ = LoadArtifactImage(host, artifact);
            }
        }
        else if (artifact.Text != null) { var box = Editor(artifact.Text, true); box.IsReadOnly = true; box.MaxHeight = 520; content.Children.Add(box); }
        else if (artifact.Base64 != null && NativeDocumentPreview.Kind(artifact.Mime) is { } kind)
        { preview = new NativeRichPreview(design, kind, artifact.Base64, artifact.Filename, escape: HandleEscape); content.Children.Add(preview); }
        else content.Children.Add(Text(L("内容类型 · ", "Content type · ") + artifact.Mime, 12, "text-faint"));
        if (artifact.Truncated) content.Children.Add(Text(L("内容已截断，当前显示部分内容。", "The content is truncated; only part is shown."), 12, "text-faint"));
    }
    private async Task LoadArtifactImage(Border host, JourneyArtifact artifact)
    {
        try
        {
            using var memory = new Windows.Storage.Streams.InMemoryRandomAccessStream();
            using (var writer = new Windows.Storage.Streams.DataWriter(memory)) { writer.WriteBytes(Convert.FromBase64String(artifact.Base64!)); await writer.StoreAsync(); writer.DetachStream(); }
            memory.Seek(0); var bitmap = new Microsoft.UI.Xaml.Media.Imaging.BitmapImage(); await bitmap.SetSourceAsync(memory);
            if (!disposed && artifactOpen && model.Artifact?.VersionId == artifact.VersionId) host.Child = new Image { Source = bitmap, Stretch = Stretch.Uniform, MaxHeight = host.MaxHeight };
        }
        catch { if (!disposed && artifactOpen) host.Child = Text(L("无法读取图像。", "Unable to read this image."), 13, "err"); }
    }
    private static readonly string[] GraphKinds = ["decision", "paper", "data_asset", "run", "artifact"];
    private string GraphKind(string kind) => design.T("graph.kind." + kind);
    private static KeyValuePair<string, string>[] Metadata(string json)
    { try { using var doc = JsonDocument.Parse(json); return doc.RootElement.ValueKind == JsonValueKind.Object ? doc.RootElement.EnumerateObject().OrderBy(p => p.Name, StringComparer.Ordinal).Select(p => KeyValuePair.Create(p.Name, p.Value.ValueKind == JsonValueKind.String ? p.Value.GetString() ?? "" : p.Value.GetRawText())).ToArray() : []; } catch (JsonException) { return []; } }
    private static string MetadataText(string json) => string.Join(" · ", Metadata(json).Select(p => p.Key + ": " + p.Value));
    private void RenderGraph()
    {
        if (!graphTab) return;
        var graph = model.Graph ?? new([], []);
        if (renderedGraph == model.Graph && renderedGraphCanvas == graphCanvas && renderedGraphBusy == model.GraphBusy && renderedGraphError == model.GraphError && graphHost.Children.Count > 0 && selectedEdge == null) return;
        renderedGraph = model.Graph; renderedGraphCanvas = graphCanvas; renderedGraphBusy = model.GraphBusy; renderedGraphError = model.GraphError;
        graphHost.Children.Clear(); graphHost.RowDefinitions.Clear(); Rows(graphHost, GridLength.Auto, new(1, GridUnitType.Star), GridLength.Auto);
        var tools = new Grid { Margin = new(0, 0, 0, 16) }; Columns(tools, new(1, GridUnitType.Star), GridLength.Auto);
        tools.Children.Add(Text(L($"{graph.Nodes.Length} 个节点 · {graph.Edges.Length} 条关系", $"{graph.Nodes.Length} nodes · {graph.Edges.Length} relationships"), 14, "text-muted"));
        Put(tools, Action(graphCanvas ? L("列表", "List") : L("图谱", "Graph"), () => { graphCanvas = !graphCanvas; renderedGraph = null; RenderGraph(); }), 0, 1); graphHost.Children.Add(tools);
        if (model.GraphBusy) { Put(graphHost, Text(L("正在读取关系图…", "Loading relationships…"), 14, "text-faint"), 1, 0); return; }
        if (model.GraphError != null) { var error = new StackPanel { Spacing = 12 }; error.Children.Add(Text(model.GraphError, 13, "err")); error.Children.Add(Link(L("重试", "Try again"), () => _ = model.LoadGraphAsync())); Put(graphHost, error, 1, 0); return; }
        var titles = graph.Nodes.ToDictionary(n => n.Id, n => n.Title);
        string Title(string id) => titles.GetValueOrDefault(id, id);
        if (graphCanvas)
        {
            var kinds = GraphKinds.Where(k => graph.Nodes.Any(n => n.Kind == k)).ToArray(); var used = Math.Max(1, kinds.Length) * 176 + Math.Max(0, kinds.Length - 1) * 64;
            var width = Math.Max(720, used + 80); var height = Math.Max(360, 92 + kinds.Select(k => graph.Nodes.Count(n => n.Kind == k)).DefaultIfEmpty().Max() * 92);
            var canvas = new Canvas { Width = width, Height = height, Background = design.Brush("bg-app") }; var positions = new Dictionary<string, Windows.Foundation.Point>();
            for (var col = 0; col < kinds.Length; col++) { var row = 0; foreach (var node in graph.Nodes.Where(n => n.Kind == kinds[col])) positions[node.Id] = new((width - used) / 2.0 + col * 240, 64 + row++ * 92); }
            foreach (var edge in graph.Edges)
            {
                if (!positions.TryGetValue(edge.SourceId, out var from) || !positions.TryGetValue(edge.TargetId, out var to)) continue;
                var route = JourneyGraphRoute.Between(from.X, from.Y, to.X, to.Y);
                static Windows.Foundation.Point Point(JourneyGraphPoint p) => new(p.X, p.Y);
                var figure = new PathFigure { StartPoint = Point(route.Start) };
                foreach (var segment in route.Segments)
                    if (segment.IsLine) figure.Segments.Add(new LineSegment { Point = Point(segment.End) });
                    else figure.Segments.Add(new BezierSegment { Point1 = Point(segment.Control1), Point2 = Point(segment.Control2), Point3 = Point(segment.End) });
                var geometry = new PathGeometry(); geometry.Figures.Add(figure);
                var path = new Microsoft.UI.Xaml.Shapes.Path { Data = geometry, Stroke = design.Brush(selectedEdge == edge ? "clay" : "border-strong"), StrokeThickness = 2 }; canvas.Children.Add(path);
                var last = route.Segments[^1]; var angle = Math.Atan2(last.End.Y - last.Control2.Y, last.End.X - last.Control2.X);
                var arrow = new Polygon { Fill = path.Stroke }; arrow.Points.Add(Point(last.End));
                arrow.Points.Add(new(last.End.X - 8 * Math.Cos(angle) + 4 * Math.Sin(angle), last.End.Y - 8 * Math.Sin(angle) - 4 * Math.Cos(angle)));
                arrow.Points.Add(new(last.End.X - 8 * Math.Cos(angle) - 4 * Math.Sin(angle), last.End.Y - 8 * Math.Sin(angle) + 4 * Math.Cos(angle))); canvas.Children.Add(arrow);
                var edgeButton = Link(edge.Relation, () => { selectedEdge = edge; RenderGraph(); }, 11); edgeButton.Background = design.Brush("bg-app");
                edgeButton.Measure(new(double.PositiveInfinity, double.PositiveInfinity)); Canvas.SetLeft(edgeButton, Math.Max(0, route.Label.X - edgeButton.DesiredSize.Width / 2)); Canvas.SetTop(edgeButton, route.Label.Y - edgeButton.DesiredSize.Height); canvas.Children.Add(edgeButton);
                AutomationProperties.SetName(edgeButton, Title(edge.SourceId) + " → " + Title(edge.TargetId) + " · " + edge.Relation);
            }
            for (var col = 0; col < kinds.Length; col++)
            {
                var label = Text(GraphKind(kinds[col]), 12, "text-muted"); label.Width = 176; label.TextAlignment = TextAlignment.Center;
                Canvas.SetLeft(label, (width - used) / 2.0 + col * 240); Canvas.SetTop(label, 24); canvas.Children.Add(label);
            }
            foreach (var node in graph.Nodes)
            {
                if (!positions.TryGetValue(node.Id, out var point)) continue;
                var content = new StackPanel { Spacing = 5 }; content.Children.Add(Text(GraphKind(node.Kind), 11, "clay-strong")); var title = Text(node.Title, 12, bold: true); title.TextWrapping = TextWrapping.NoWrap; title.TextTrimming = TextTrimming.CharacterEllipsis; content.Children.Add(title);
                var card = new Border { Child = content, Width = 176, Height = 58, Padding = new(12, 8, 12, 8), CornerRadius = new(10), Background = design.Brush("bg-elev"), BorderBrush = design.Brush("clay"), BorderThickness = new(1) }; ToolTipService.SetToolTip(card, GraphKind(node.Kind) + ": " + node.Title); Canvas.SetLeft(card, point.X); Canvas.SetTop(card, point.Y); canvas.Children.Add(card);
            }
            var scroll = new ScrollViewer { Content = canvas, HorizontalScrollBarVisibility = ScrollBarVisibility.Auto, HorizontalScrollMode = ScrollMode.Enabled, VerticalScrollBarVisibility = ScrollBarVisibility.Auto }; Put(graphHost, scroll, 1, 0);
        }
        else
        {
            var list = new StackPanel { Spacing = 18 };
            foreach (var kind in GraphKinds)
            {
                var nodes = graph.Nodes.Where(n => n.Kind == kind).ToArray(); if (nodes.Length == 0) continue;
                list.Children.Add(Text(GraphKind(kind) + "   " + nodes.Length, 15, bold: true));
                foreach (var node in nodes)
                {
                    var content = new StackPanel { Spacing = 6 }; content.Children.Add(Text(node.Title, 14, bold: true));
                    if (!string.IsNullOrEmpty(node.RefId)) content.Children.Add(Text(node.RefId, 11, "text-faint"));
                    var meta = MetadataText(node.MetadataJson); if (meta.Length > 0) content.Children.Add(Text(meta, 11, "text-faint"));
                    foreach (var edge in graph.Edges.Where(e => e.SourceId == node.Id))
                    { content.Children.Add(Link(edge.Relation + "  " + Title(edge.TargetId), () => { selectedEdge = edge; RenderGraph(); })); var edgeMeta = MetadataText(edge.MetadataJson); if (edgeMeta.Length > 0) content.Children.Add(Text(edgeMeta, 11, "text-faint")); }
                    list.Children.Add(new Border { Child = content, Padding = new(16), BorderBrush = design.Brush("border"), BorderThickness = new(1), Background = design.Brush("bg-elev"), CornerRadius = new(8) });
                }
            }
            Put(graphHost, Scroll(list), 1, 0);
        }
        if (selectedEdge is { } selected)
        {
            var detail = new StackPanel { Spacing = 8 }; var head = new Grid(); Columns(head, new(1, GridUnitType.Star), GridLength.Auto); head.Children.Add(Text(selected.Relation + " · " + Title(selected.SourceId) + " → " + Title(selected.TargetId), 14, bold: true));
            Put(head, Icon(design.T("graph.edge.close"), "close", () => { selectedEdge = null; renderedGraph = null; RenderGraph(); }), 0, 1); detail.Children.Add(head);
            var pairs = Metadata(selected.MetadataJson); if (pairs.Length == 0) detail.Children.Add(Text(design.T("graph.edge.no_metadata"), 12, "text-faint"));
            foreach (var pair in pairs) detail.Children.Add(Text(pair.Key + ": " + pair.Value, 12));
            Put(graphHost, new Border { Child = detail, Padding = new(16), Margin = new(0, 16, 0, 0), Background = design.Brush("bg-elev"), BorderBrush = design.Brush("border"), BorderThickness = new(1), CornerRadius = new(8) }, 2, 0);
        }
    }
}
