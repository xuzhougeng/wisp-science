using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed class NativePublicationPage : NativeActionPage
{
    private readonly WorkspacePublicationModel model;
    private readonly StackPanel creation = new() { Spacing = 10 }, evidence = new() { Spacing = 10 }, editor = new() { Spacing = 10 },
        sourcePanel = new() { Spacing = 10 }, sourcePicker = new() { Spacing = 10 }, sourceRows = new() { Spacing = 6 }, sourcePreview = new() { Spacing = 8 },
        readinessPanel = new() { Spacing = 10 }, readinessRows = new() { Spacing = 8 }, versions = new() { Spacing = 10 };
    private readonly ComboBox papers = new() { Header = "论文", HorizontalAlignment = HorizontalAlignment.Stretch }, revisions = new() { Header = "版本", HorizontalAlignment = HorizontalAlignment.Stretch };
    private readonly TextBlock summary = new() { TextWrapping = TextWrapping.Wrap };
    private readonly TextBlock uncertain = new() { Text = "上次操作结果未确认。请先刷新核对记录，再确认已核对；不会自动重复提交。", TextWrapping = TextWrapping.Wrap };
    private readonly Button acknowledge;
    private readonly List<ComboBox> selectors = [];
    private NativePublicationWorkspace? renderedWorkspace;
    private NativePublicationSources? renderedSources;
    private NativePublicationSource? renderedSource;
    private JsonObject? renderedReadiness;
    private string tab = "evidence", sourceKind = "files", sourceQuery = "";
    private uint sourceOffset;
    private bool rendering, editing, confirmingFreeze;
    private bool renderedSourcesLoading;
    private string? checkedPolicy;
    private readonly TextBox newTitle, newDescription, newLabel, cloneLabel, destination;
    private readonly ComboBox visibility;
    private readonly CheckBox pii = new() { Content = "已检查个人或敏感信息" }, redistribution = new() { Content = "已检查再分发权限" }, restricted = new() { Content = "包含受限来源的快照字节" };
    private readonly Button freeze, clone, buildCapsule, checkRevision;
    private readonly NativeActionWrap sourcePaging = new();

    public NativePublicationPage(WorkspacePublicationModel model, WispDesign design, Func<Task<string?>> pickDirectory, Action close) : base(design, "论文证据", model, close)
    {
        this.model = model;
        var refresh = Design.ToolButton("刷新论文工作区", "refresh"); refresh.Click += async (_, _) => await model.LoadAsync(); HeaderActions.Children.Add(refresh);
        Form.Children.Add(uncertain);
        acknowledge = Button("已刷新并核对操作结果", () => { model.AcknowledgeUncertain(); return Task.CompletedTask; }); Form.Children.Add(acknowledge);
        Form.Children.Add(papers); Form.Children.Add(revisions); Form.Children.Add(summary);
        TrackSelector(papers); TrackSelector(revisions);
        papers.SelectionChanged += async (_, _) => { if (!rendering && papers.SelectedItem is ComboBoxItem i) await model.SelectWorkspaceAsync((string)i.Tag, null); };
        revisions.SelectionChanged += async (_, _) => { if (!rendering && revisions.SelectedItem is ComboBoxItem i) await model.SelectWorkspaceAsync(model.PublicationId, (string)i.Tag); };
        newTitle = Input(creation, "论文标题", model.Title, text => model.Title = text);
        newDescription = Input(creation, "描述", model.Description, text => model.Description = text, true);
        newLabel = Input(creation, "版本标签", model.RevisionLabel, text => model.RevisionLabel = text);
        creation.Children.Add(Button("创建论文证据", async () => { if (await model.CreateAsync()) newTitle.Text = newDescription.Text = newLabel.Text = ""; }));
        Form.Children.Add(creation);
        var tabs = new NativeActionWrap();
        foreach (var (id, label) in new[] { ("evidence", "证据与条目"), ("sources", "添加证据"), ("readiness", "冻结检查"), ("versions", "版本与复现") })
            tabs.Children.Add(Button(label, async () => { tab = id; ShowTab(); if (id == "sources") await LoadSources(); }));
        Form.Children.Add(tabs);
        Results.Children.Add(evidence); Results.Children.Add(editor); Results.Children.Add(sourcePanel); Results.Children.Add(readinessPanel); Results.Children.Add(versions);
        SetupSources();
        visibility = Choice(readinessPanel, "冻结包的可见性", [ ("private", "私有"), ("restricted", "受限"), ("public", "公开") ], "private", _ => InvalidateCheck());
        foreach (var check in new[] { pii, redistribution, restricted }) { readinessPanel.Children.Add(check); check.Checked += (_, _) => InvalidateCheck(); check.Unchecked += (_, _) => InvalidateCheck(); }
        checkRevision = Button("检查当前版本", async () => {
            confirmingFreeze = false; var policy = Policy();
            if (await model.MutateAsync(new() { ["action"] = "check", ["policy"] = policy })) { checkedPolicy = policy.ToJsonString(); Render(); }
        }); readinessPanel.Children.Add(checkRevision);
        freeze = Button("冻结此版本", async () => {
            if (!confirmingFreeze) { confirmingFreeze = true; freeze!.Content = "确认冻结：此版本将不可编辑"; return; }
            confirmingFreeze = false; await model.MutateAsync(new() { ["action"] = "freeze", ["policy"] = Policy() });
        }); readinessPanel.Children.Add(freeze); readinessPanel.Children.Add(readinessRows);
        cloneLabel = Input(versions, "新版本标签", "", _ => { });
        clone = Button("从当前版本创建修订", async () => { if (!string.IsNullOrWhiteSpace(cloneLabel.Text)) await model.MutateAsync(new() { ["action"] = "clone_revision", ["label"] = cloneLabel.Text.Trim() }); }); versions.Children.Add(clone);
        destination = Input(versions, "证据包保存路径（新的 .zip 文件）", "", _ => { });
        versions.Children.Add(Button("选择保存目录", async () => {
            var revision = model.RevisionId;
            var directory = await pickDirectory();
            if (directory != null && !model.Closed && revision == model.RevisionId) {
                var name = System.IO.Path.GetFileName(destination.Text.Trim());
                if (string.IsNullOrWhiteSpace(name)) name = "publication-" + revision + ".zip";
                destination.Text = System.IO.Path.Combine(directory, name);
            }
        }));
        versions.Children.Add(Mute("冻结版本后可构建证据包。请选择新的 ZIP 文件名，已有文件不会被覆盖。"));
        buildCapsule = Button("构建证据包", async () => { if (!string.IsNullOrWhiteSpace(destination.Text)) await model.MutateAsync(new() { ["action"] = "build_capsule", ["destination"] = destination.Text.Trim() }); }); versions.Children.Add(buildCapsule);
        model.Changed += Render; Render(); _ = model.LoadAsync();
    }
    private TextBox Input(Panel parent, string label, string text, Action<string> change, bool multiline = false)
    {
        var field = new TextBox { Header = label, AcceptsReturn = multiline, TextWrapping = TextWrapping.Wrap, MinHeight = multiline ? 90 : 32, MaxHeight = multiline ? 260 : double.PositiveInfinity };
        field.Text = text; field.TextChanged += (_, _) => change(field.Text); parent.Children.Add(field); return field;
    }
    private ComboBox Choice(Panel parent, string label, IEnumerable<(string Id, string Label)> options, string? selected, Action<string> change)
    {
        var combo = new ComboBox { Header = label, HorizontalAlignment = HorizontalAlignment.Stretch };
        foreach (var (id, title) in options) combo.Items.Add(new ComboBoxItem { Tag = id, Content = title });
        combo.SelectedItem = combo.Items.Cast<ComboBoxItem>().FirstOrDefault(i => (string)i.Tag == (selected ?? ""));
        combo.SelectionChanged += (_, _) => { if (combo.SelectedItem is ComboBoxItem i) change((string)i.Tag); };
        parent.Children.Add(combo); TrackSelector(combo); return combo;
    }
    private void TrackSelector(ComboBox combo)
    {
        combo.Loaded += (_, _) => { if (!selectors.Contains(combo)) selectors.Add(combo); };
        combo.Unloaded += (_, _) => selectors.Remove(combo);
    }
    private static string Selected(ComboBox combo) => (combo.SelectedItem as ComboBoxItem)?.Tag as string ?? "";
    private JsonObject Policy() => new() { ["target_visibility"] = Selected(visibility), ["phi_pii_reviewed"] = pii.IsChecked == true,
        ["redistribution_reviewed"] = redistribution.IsChecked == true, ["snapshot_restricted_bytes"] = restricted.IsChecked == true };
    private void InvalidateCheck() { checkedPolicy = null; confirmingFreeze = false; if (freeze != null) { freeze.Content = "冻结此版本"; freeze.IsEnabled = false; } }
    private void ShowTab()
    {
        evidence.Visibility = tab == "evidence" && !editing ? Visibility.Visible : Visibility.Collapsed;
        editor.Visibility = tab == "evidence" && editing ? Visibility.Visible : Visibility.Collapsed;
        sourcePanel.Visibility = tab == "sources" ? Visibility.Visible : Visibility.Collapsed;
        sourcePicker.Visibility = model.Source == null ? Visibility.Visible : Visibility.Collapsed;
        sourcePreview.Visibility = model.Source != null ? Visibility.Visible : Visibility.Collapsed;
        readinessPanel.Visibility = tab == "readiness" ? Visibility.Visible : Visibility.Collapsed;
        versions.Visibility = tab == "versions" ? Visibility.Visible : Visibility.Collapsed;
    }
    private void Render()
    {
        if (model.Closed) return;
        uncertain.Visibility = acknowledge.Visibility = model.MutationUncertain ? Visibility.Visible : Visibility.Collapsed;
        acknowledge.IsEnabled = model.Reconciled && !model.Busy;
        creation.Visibility = model.CreationAvailable ? Visibility.Visible : Visibility.Collapsed;
        if (!ReferenceEquals(renderedWorkspace, model.Workspace))
        {
            var changedRevision = renderedWorkspace?.Revision?.Id != model.RevisionId;
            renderedWorkspace = model.Workspace; rendering = true;
            try {
                papers.Items.Clear(); revisions.Items.Clear();
                foreach (var p in model.Workspace?.Publications ?? []) papers.Items.Add(new ComboBoxItem { Tag = p.Id, Content = p.Title });
                foreach (var r in model.Workspace?.Revisions ?? []) revisions.Items.Add(new ComboBoxItem { Tag = r.Id, Content = r.Label + " · " + r.State });
                papers.SelectedItem = papers.Items.Cast<ComboBoxItem>().FirstOrDefault(i => (string)i.Tag == model.PublicationId);
                revisions.SelectedItem = revisions.Items.Cast<ComboBoxItem>().FirstOrDefault(i => (string)i.Tag == model.RevisionId);
            } finally { rendering = false; }
            summary.Text = model.Workspace?.Publication == null ? "创建论文记录，组织条目和来源证据。" : $"{model.Workspace.Publication.Description}\n{model.Workspace.Revision?.State} · {model.Workspace.Items.Count} 个条目 · 能力：{model.Workspace.EffectiveCapabilityLevel ?? model.Workspace.Revision?.CapabilityLevel}";
            if (changedRevision) { editing = false; InvalidateCheck(); }
            InvalidateCheck(); RenderEvidence(); RenderVersions(); RenderReadiness();
        }
        if (!ReferenceEquals(renderedSources, model.Sources) || model.SourcesError != null || renderedSourcesLoading != model.SourcesLoading) {
            renderedSources = model.Sources; renderedSourcesLoading = model.SourcesLoading; RenderSourceRows();
        }
        if (!ReferenceEquals(renderedSource, model.Source)) {
            renderedSource = model.Source; RenderSourcePreview();
            var selected = model.Source;
            DispatcherQueue.TryEnqueue(() => {
                if (!model.Closed && tab == "sources" && ReferenceEquals(model.Source, selected))
                    (selected == null ? sourcePicker : sourcePreview).StartBringIntoView(new() { AnimationDesired = false, VerticalAlignmentRatio = 0 });
            });
        }
        if (!ReferenceEquals(renderedReadiness, model.Readiness)) { renderedReadiness = model.Readiness; RenderReadiness(); }
        freeze.IsEnabled = model.Editable && checkedPolicy == Policy().ToJsonString() && model.Readiness?["can_freeze"]?.GetValue<bool>() == true;
        checkRevision.IsEnabled = model.Editable;
        clone.IsEnabled = model.RevisionId != null && !model.Busy && !model.MutationUncertain;
        buildCapsule.IsEnabled = model.CanBuildCapsule;
        ShowTab(); Update();
    }
    private void RenderEvidence()
    {
        evidence.Children.Clear();
        var add = Button("新增条目", () => { model.SelectItem(null); editing = true; RenderEditor(); ShowTab(); return Task.CompletedTask; }); add.IsEnabled = model.Workspace?.Revision?.State == "draft"; evidence.Children.Add(add);
        foreach (var item in (model.Workspace?.Items ?? []).OrderBy(i => i.Ordinal))
        {
            var row = new StackPanel { Spacing = 7 };
            row.Children.Add(Design.Text(item.Title, 18)); row.Children.Add(Mute($"{KindLabel(item.Kind)} · 顺序 {item.Ordinal}"));
            if (item.Content.Length > 0) row.Children.Add(new TextBlock { Text = item.Content, TextWrapping = TextWrapping.Wrap, IsTextSelectionEnabled = true });
            var edit = Button("编辑条目", () => { model.SelectItem(item.Id); editing = true; RenderEditor(); ShowTab(); return Task.CompletedTask; }); edit.IsEnabled = model.Workspace?.Revision?.State == "draft"; row.Children.Add(edit);
            foreach (var binding in (model.Workspace?.Bindings ?? []).Where(b => b.ItemId == item.Id)) row.Children.Add(BindingCard(binding));
            evidence.Children.Add(Design.Card(row));
        }
        foreach (var binding in (model.Workspace?.Bindings ?? []).Where(b => b.ItemId == null)) evidence.Children.Add(BindingCard(binding));
        foreach (var link in model.Workspace?.ItemLinks ?? []) {
            string Title(string key) => model.Workspace?.Items.FirstOrDefault(i => i.Id == link[key]?.GetValue<string>())?.Title ?? link[key]?.ToString() ?? "";
            evidence.Children.Add(Mute($"条目关系：{Title("source_item_id")} · {link["relation"]} · {Title("target_item_id")}"));
        }
    }
    private static string KindLabel(string kind) => kind switch { "section" => "章节", "claim" => "结论", "figure" => "图", "table" => "表", "methods" => "方法", "supplement" => "补充材料", _ => kind };
    private void RenderEditor()
    {
        editor.Children.Clear(); var draft = model.ItemDraft;
        Choice(editor, "条目类型", new[] { "section", "claim", "figure", "table", "methods", "supplement" }.Select(k => (k, KindLabel(k))), draft.Kind, s => model.ItemDraft = model.ItemDraft with { Kind = s });
        Input(editor, "条目标题", draft.Title, s => model.ItemDraft = model.ItemDraft with { Title = s });
        Input(editor, "正文／说明", draft.Content, s => model.ItemDraft = model.ItemDraft with { Content = s }, true);
        Choice(editor, "所属条目", new[] { ("", "无上级") }.Concat((model.Workspace?.Items ?? []).Where(i => i.Id != draft.Id).Select(i => (i.Id, i.Title))), draft.ParentId, s => model.ItemDraft = model.ItemDraft with { ParentId = s.Length == 0 ? null : s });
        Input(editor, "排序（非负整数）", draft.Ordinal.ToString(), s => model.ItemDraft = model.ItemDraft with { Ordinal = long.TryParse(s, out var n) ? n : -1 });
        editor.Children.Add(Button("保存条目", async () => { if (await model.SaveItemAsync()) { editing = false; RenderEvidence(); ShowTab(); } }));
        editor.Children.Add(Button("返回证据列表", () => { editing = false; ShowTab(); return Task.CompletedTask; }));
    }
    private FrameworkElement BindingCard(NativePublicationBinding b)
    {
        var body = new StackPanel { Spacing = 6 };
        var lineage = model.Workspace?.Lineage?.FirstOrDefault(v => v["binding_id"]?.GetValue<string>() == b.Id);
        body.Children.Add(Design.Text(lineage?["source_label"]?.GetValue<string>() ?? b.SourceKind, 15));
        body.Children.Add(Mute(b.Purpose + "\n" + b.SelectionState + " · " + b.ReviewState + " · " + b.ReproductionState));
        if (lineage != null) body.Children.Add(Mute($"来源质量：{lineage["quality"]} · 版本：{lineage["version_number"]}\n运行：{lineage["producing_run_title"]}\n校验：{lineage["checksum"]}"));
        if (model.Workspace?.Drift?.Any(d => d["binding_id"]?.GetValue<string>() == b.Id && d["has_drift"]?.GetValue<bool>() == true) == true) body.Children.Add(Warn("来源已有新版本；此证据仍绑定原始版本。"));
        if (model.Workspace?.Revision?.State == "draft")
        {
            var selection = Choice(body, "证据状态", NativePublicationEvidence.SelectionChoices, b.SelectionState, _ => { });
            var visible = Choice(body, "来源可见性", [("private", "私有"), ("restricted", "受限"), ("public", "公开")], b.Visibility, _ => { });
            body.Children.Add(Button("保存证据状态", async () => await model.MutateAsync(new() { ["action"] = "update_binding", ["binding_id"] = b.Id, ["selection_state"] = Selected(selection), ["visibility"] = Selected(visible) })));
        }
        else if (model.Workspace?.Revision?.State is "frozen" or "published")
        {
            var run = b.SourceKind == "run" ? b.SourceId : lineage?["producing_run_id"]?.GetValue<string>();
            if (run != null) body.Children.Add(Button("在隔离目录验证复现", async () => await model.MutateAsync(new() { ["action"] = "verify", ["source_run_id"] = run, ["comparisons"] = new JsonArray() })));
        }
        var source = new TextBox { AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, IsReadOnly = true, MaxHeight = 220 }; source.Text = b.SourceSnapshotJson;
        body.Children.Add(Disclosure("精确来源与快照", source));
        foreach (var review in (model.Workspace?.Reviews ?? []).Where(r => r["binding_id"]?.GetValue<string>() == b.Id))
            body.Children.Add(Mute($"核验：{review["reviewer"]} · {review["method"]} · {review["result"]}\n{review["report_json"]}"));
        foreach (var supersession in (model.Workspace?.Supersessions ?? []).Where(r => r["old_binding_id"]?.GetValue<string>() == b.Id || r["new_binding_id"]?.GetValue<string>() == b.Id))
            body.Children.Add(Mute($"证据替代：{supersession["old_binding_id"]} → {supersession["new_binding_id"]}\n{supersession["reason"]}"));
        return Design.Card(body);
    }
    private void SetupSources()
    {
        sourcePanel.Children.Add(sourcePicker);
        Choice(sourcePicker, "来源类型", [("files", "文件版本"), ("runs", "运行"), ("messages", "消息片段")], sourceKind, s => { sourceKind = s; sourceOffset = 0; _ = LoadSources(); });
        Input(sourcePicker, "查找来源", "", s => sourceQuery = s);
        sourcePicker.Children.Add(Button("搜索来源", async () => { sourceOffset = 0; await LoadSources(); }));
        SetupPreciseSources();
        sourcePicker.Children.Add(new ScrollViewer { Content = sourceRows, MaxHeight = 300, VerticalScrollBarVisibility = ScrollBarVisibility.Auto, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled });
        sourcePicker.Children.Add(sourcePaging); sourcePanel.Children.Add(sourcePreview);
    }
    private void SetupPreciseSources()
    {
        var form = new StackPanel { Spacing = 8 };
        var messageFields = new StackPanel { Spacing = 8 };
        var rangeFields = new StackPanel { Spacing = 8 };
        var callFields = new StackPanel { Spacing = 8 };
        var exactFields = new StackPanel { Spacing = 8 };
        void SelectKind(string value) {
            messageFields.Visibility = value is "message_span" or "tool_call" ? Visibility.Visible : Visibility.Collapsed;
            rangeFields.Visibility = value == "message_span" ? Visibility.Visible : Visibility.Collapsed;
            callFields.Visibility = value == "tool_call" ? Visibility.Visible : Visibility.Collapsed;
            exactFields.Visibility = value is "execution_log" or "code_cell" or "external_resource" ? Visibility.Visible : Visibility.Collapsed;
        }
        var kind = Choice(form, "精确来源类型", [("message_span", "消息片段"), ("tool_call", "工具调用"), ("execution_log", "执行日志"), ("code_cell", "代码单元"), ("external_resource", "外部资源")], "message_span", SelectKind);
        form.Children.Add(messageFields); form.Children.Add(rangeFields); form.Children.Add(callFields); form.Children.Add(exactFields);
        var frame = Input(messageFields, "会话 ID", "", _ => { });
        var sequence = Input(messageFields, "消息序号", "", _ => { });
        var start = Input(rangeFields, "起始 UTF-8 字节（含）", "0", _ => { });
        var end = Input(rangeFields, "结束 UTF-8 字节（不含）", "", _ => { });
        var call = Input(callFields, "工具调用 ID", "", _ => { });
        var exact = Input(exactFields, "精确来源 ID", "", _ => { });
        form.Children.Add(Button("继续设置证据用途", () => {
            try { model.SelectPreciseSource(Selected(kind), frame.Text, sequence.Text, start.Text, end.Text, call.Text, exact.Text); }
            catch (Exception e) { model.Fail(e.Message); }
            return Task.CompletedTask;
        }));
        SelectKind("message_span"); sourcePicker.Children.Add(Disclosure("添加精确来源", form));
    }
    private Task LoadSources() => model.LoadSourcesAsync(sourceKind, sourceQuery, sourceOffset);
    private void RenderSourceRows()
    {
        sourceRows.Children.Clear(); sourcePaging.Children.Clear();
        if (model.SourcesError != null) sourceRows.Children.Add(Warn(model.SourcesError));
        if (model.SourcesLoading) sourceRows.Children.Add(Mute("正在读取来源…"));
        foreach (var source in model.Sources.Sources) {
            var label = NativePublicationEvidence.SourceLabel(source);
            var choice = Button(label, () => { model.SelectSource(source); return Task.CompletedTask; });
            choice.Content = new TextBlock { Text = label, TextWrapping = TextWrapping.Wrap };
            choice.HorizontalAlignment = choice.HorizontalContentAlignment = HorizontalAlignment.Stretch;
            sourceRows.Children.Add(choice);
        }
        var prev = Button("上一页来源", async () => { sourceOffset = sourceOffset >= 50 ? sourceOffset - 50 : 0; await LoadSources(); }); prev.IsEnabled = sourceOffset > 0 && !model.SourcesLoading;
        var next = Button("下一页来源", async () => { sourceOffset += 50; await LoadSources(); }); next.IsEnabled = model.Sources.HasMore && !model.SourcesLoading;
        sourcePaging.Children.Add(prev); sourcePaging.Children.Add(next);
    }
    private void RenderSourcePreview()
    {
        sourcePreview.Children.Clear(); if (model.Source is not { } source) return;
        sourcePreview.Children.Add(Button("返回选择来源", () => { model.ClearSource(); return Task.CompletedTask; }));
        sourcePreview.Children.Add(Design.Text(source.Title, 18)); sourcePreview.Children.Add(Mute("绑定此精确来源；后续来源变化不会替换已登记证据。"));
        if (source.Text == null) sourcePreview.Children.Add(Mute(source.Id));
        TextBox? excerpt = null; int start = 0, length = 0;
        if (source.Text != null) {
            excerpt = new TextBox { AcceptsReturn = true, IsReadOnly = true, TextWrapping = TextWrapping.Wrap, MaxHeight = 240, Header = "选择消息片段（未选择则使用显示的完整内容）" }; excerpt.Text = source.Text;
            excerpt.SelectionChanged += (_, _) => { start = excerpt.SelectionStart; length = excerpt.SelectionLength; }; sourcePreview.Children.Add(excerpt);
        }
        var item = Choice(sourcePreview, "绑定到条目", new[] { ("", "整个版本") }.Concat((model.Workspace?.Items ?? []).Select(i => (i.Id, i.Title))), model.SelectedItem, _ => { });
        var purpose = Input(sourcePreview, "用途", "", _ => { }, true);
        var claim = Choice(sourcePreview, "支持的结论", new[] { ("", "不指定") }.Concat((model.Workspace?.Items ?? []).Where(i => i.Kind == "claim").Select(i => (i.Id, i.Title))), "", _ => { });
        var state = Choice(sourcePreview, "证据状态", NativePublicationEvidence.SelectionChoices, "selected", _ => { });
        var visible = Choice(sourcePreview, "来源可见性", [("private", "私有"), ("restricted", "受限"), ("public", "公开")], "private", _ => { });
        var bind = Button("绑定此来源", async () => {
            try { if (await model.BindSourceAsync(Empty(Selected(item)), purpose.Text, Empty(Selected(claim)), Selected(state), Selected(visible), start, length)) { tab = "evidence"; ShowTab(); } }
            catch (Exception e) { model.Fail(e.Message); }
        }); bind.IsEnabled = model.Editable; sourcePreview.Children.Add(bind);
    }
    private static string? Empty(string s) => s.Length == 0 ? null : s;
    private void RenderReadiness()
    {
        readinessRows.Children.Clear(); freeze.Content = "冻结此版本"; confirmingFreeze = false;
        foreach (var waiver in model.Workspace?.Waivers ?? [])
            readinessRows.Children.Add(Mute($"例外说明：{waiver["finding_code"]} · {waiver["author"]}\n{waiver["reason"]}"));
        if (model.Readiness is not { } value) {
            if (model.Workspace?.Revision?.State == "draft") readinessRows.Children.Add(Mute("请检查当前版本。保存条目、证据或例外说明后，需要重新检查才能冻结。"));
            return;
        }
        readinessRows.Children.Add(Mute($"能力：{value["capability_level"]} · 可以冻结：{value["can_freeze"]}\n清单校验：{value["manifest_sha256"]}"));
        foreach (var key in new[] { "blockers", "warnings", "omissions" }) foreach (var row in value[key]?.AsArray() ?? [])
        {
            if (row is not JsonObject finding) continue;
            var body = new StackPanel { Spacing = 6 }; body.Children.Add(Warn($"{finding["code"]} · {finding["message"]}"));
            if (finding["waivable"]?.GetValue<bool>() == true && finding["waived"]?.GetValue<bool>() != true && model.Workspace?.Revision?.State == "draft") {
                var author = Input(body, "说明人", "Local user", _ => { }); var reason = Input(body, "保留此限制的理由", "", _ => { }, true);
                body.Children.Add(Button("记录例外说明", async () => { if (!string.IsNullOrWhiteSpace(reason.Text)) await model.MutateAsync(new() { ["action"] = "save_waiver", ["finding_code"] = finding["code"]?.GetValue<string>(), ["author"] = author.Text, ["reason"] = reason.Text }); }));
            }
            readinessRows.Children.Add(Design.Card(body));
        }
    }
    private readonly StackPanel versionRows = new() { Spacing = 8 };
    private void RenderVersions()
    {
        if (!versions.Children.Contains(versionRows)) versions.Children.Add(versionRows);
        versionRows.Children.Clear();
        foreach (var r in model.Workspace?.Revisions ?? []) versionRows.Children.Add(Mute($"{r.Label} · {r.State} · {r.CapabilityLevel}\n{r.ManifestSha256}"));
        foreach (var build in model.Workspace?.CapsuleBuilds ?? []) versionRows.Children.Add(Mute($"证据包 · {build["status"]}\n{build["output_path"]}\nSHA256：{build["archive_sha256"]}\n{build["error"]}"));
        foreach (var run in model.Workspace?.ReproductionRuns ?? []) {
            var body = new StackPanel { Spacing = 6 }; body.Children.Add(Mute($"复现 {run["source_run_id"]} · {run["status"]} · {run["capability_level"]}\n环境匹配：{run["environment_matched"]} · 退出码：{run["exit_code"]}\n{run["error"]}"));
            var output = new TextBox { AcceptsReturn = true, IsReadOnly = true, TextWrapping = TextWrapping.Wrap, MaxHeight = 200 }; output.Text = $"{run["stdout_tail"]}\n{run["stderr_tail"]}"; body.Children.Add(output);
            foreach (var result in (model.Workspace?.ReproductionResults ?? []).Where(v => v["reproduction_run_id"]?.GetValue<string>() == run["id"]?.GetValue<string>()))
                body.Children.Add(Mute($"{result["output_path"]} · {result["comparator_kind"]} · 通过：{result["passed"]}\n{result["report_json"]}"));
            versionRows.Children.Add(Design.Card(body));
        }
    }
    public override void HandleEscape()
    {
        foreach (var selector in selectors.AsEnumerable().Reverse()) if (selector.IsDropDownOpen) { selector.IsDropDownOpen = false; return; }
        if (confirmingFreeze) { InvalidateCheck(); return; }
        if (editing) { editing = false; ShowTab(); return; }
        if (tab == "sources" && model.Source != null) { model.ClearSource(); return; }
        if (tab != "evidence") { tab = "evidence"; ShowTab(); return; }
        base.HandleEscape();
    }
    public override void Dispose() { model.Changed -= Render; base.Dispose(); }
}
