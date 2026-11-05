using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class NativePublicationPage : NativeActionPage
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

    private readonly Button createPublication;
    private readonly NativeActionWrap tabs = new();
    private readonly Grid createLayout = new() { RowSpacing = 28, ColumnSpacing = 48 };
    public NativePublicationPage(WorkspacePublicationModel model, WispDesign design, Func<Task<string?>> pickDirectory, Action close) : base(design, design.T("publication.title"), model, close)
    {
        this.model = model;
        SetupPublicationHeader(close);
        Form.Children.Add(uncertain);
        acknowledge = Button("已刷新并核对操作结果", () => { model.AcknowledgeUncertain(); return Task.CompletedTask; }); Form.Children.Add(acknowledge);
        Form.Children.Add(papers); Form.Children.Add(revisions); Form.Children.Add(summary);
        TrackSelector(papers); TrackSelector(revisions);
        papers.SelectionChanged += async (_, _) => { if (!rendering && papers.SelectedItem is ComboBoxItem i) await model.SelectWorkspaceAsync((string)i.Tag, null); };
        revisions.SelectionChanged += async (_, _) => { if (!rendering && revisions.SelectedItem is ComboBoxItem i) await model.SelectWorkspaceAsync(model.PublicationId, (string)i.Tag); };
        papers.Header = design.T("publication.paper"); revisions.Header = design.T("publication.revision");
        if (model.RevisionLabel.Length == 0) model.RevisionLabel = design.T("publication.initial_revision");
        newTitle = Input(creation, design.T("publication.paper_title"), model.Title, text => { model.Title = text; UpdateCreate(); });
        newDescription = Input(creation, design.T("publication.description"), model.Description, text => model.Description = text, true);
        newDescription.MinHeight = 140;
        newLabel = Input(creation, design.T("publication.revision_label"), model.RevisionLabel, text => { model.RevisionLabel = text; UpdateCreate(); });
        createPublication = Button(design.T("publication.create_action"), async () => { if (await model.CreateAsync()) newTitle.Text = newDescription.Text = ""; }, true);
        createPublication.HorizontalAlignment = HorizontalAlignment.Stretch; creation.Children.Add(createPublication);
        SetupCreation(); Form.Children.Add(createLayout);
        foreach (var (id, key) in new[] { ("evidence", "tab_evidence"), ("sources", "add_evidence"), ("readiness", "tab_readiness"), ("versions", "tab_versions") })
            tabs.Children.Add(Button(design.T("publication." + key), async () => { model.DismissRunBinding(); tab = id; ShowTab(); if (id == "sources") await LoadSources(); }));
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
        Results.Children.Insert(0, runBinding);
        model.Changed += Render; Render(); _ = ReadPublication();
    }
    private TextBox Input(Panel parent, string label, string text, Action<string> change, bool multiline = false)
    {
        var group = new StackPanel { Spacing = 4 };
        var caption = Design.Text(label, 11); caption.Foreground = Design.Brush("text-muted"); group.Children.Add(caption);
        var field = new TextBox { AcceptsReturn = multiline, TextWrapping = TextWrapping.Wrap, MinHeight = multiline ? 90 : 36,
            MaxHeight = multiline ? 260 : double.PositiveInfinity, CornerRadius = new(7), Padding = new(10, 7, 10, 7),
            Background = Design.Brush("bg-input"), Foreground = Design.Brush("text"), BorderBrush = Design.Brush("border-strong"), BorderThickness = new(1) };
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(field, label);
        field.Text = text; field.TextChanged += (_, _) => change(field.Text); group.Children.Add(NativeDialogVisuals.Input(field, Design));
        parent.Children.Add(group); return field;
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
        sourcePanel.Visibility = tab == "sources" && model.PendingRun == null ? Visibility.Visible : Visibility.Collapsed;
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
        createLayout.Visibility = model.CreationAvailable ? Visibility.Visible : Visibility.Collapsed;
        var hasPaper = model.Workspace?.Publication != null;
        tabs.Visibility = papers.Visibility = revisions.Visibility = summary.Visibility = hasPaper ? Visibility.Visible : Visibility.Collapsed;
        Results.Visibility = hasPaper || model.PendingRun != null ? Visibility.Visible : Visibility.Collapsed;
        UpdateCreate();
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
            summary.Text = model.Workspace?.Publication == null ? "" : string.Join(" · ", new[] { model.Workspace.Publication.Description, ValueLabel("state", model.Workspace.Revision?.State), Design.L($"{model.Workspace.Items.Count} 个条目", $"{model.Workspace.Items.Count} items"), ValueLabel("capability", model.Workspace.EffectiveCapabilityLevel ?? model.Workspace.Revision?.CapabilityLevel) }.Where(s => !string.IsNullOrEmpty(s)));
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
        RenderRunBinding(); ShowTab(); Update();
    }
    private void RenderEvidence()
    {
        evidence.Children.Clear();
        var add = Button(T("add_item"), () => { model.SelectItem(null); editing = true; RenderEditor(); ShowTab(); return Task.CompletedTask; }); add.IsEnabled = model.Workspace?.Revision?.State == "draft"; evidence.Children.Add(add);
        foreach (var item in (model.Workspace?.Items ?? []).OrderBy(i => i.Ordinal))
        {
            var row = new StackPanel { Spacing = 7 };
            row.Children.Add(Design.Text(item.Title, 18)); row.Children.Add(Mute($"{KindLabel(item.Kind)} · {Design.L("顺序", "Order")} {item.Ordinal}"));
            if (item.Content.Length > 0) row.Children.Add(new TextBlock { Text = item.Content, TextWrapping = TextWrapping.Wrap, IsTextSelectionEnabled = true });
            var edit = Button(Design.L("编辑条目", "Edit item"), () => { model.SelectItem(item.Id); editing = true; RenderEditor(); ShowTab(); return Task.CompletedTask; }); edit.IsEnabled = model.Workspace?.Revision?.State == "draft"; row.Children.Add(edit);
            foreach (var binding in (model.Workspace?.Bindings ?? []).Where(b => b.ItemId == item.Id)) row.Children.Add(BindingCard(binding));
            evidence.Children.Add(Design.Card(row));
        }
        foreach (var binding in (model.Workspace?.Bindings ?? []).Where(b => b.ItemId == null)) evidence.Children.Add(BindingCard(binding));
        foreach (var link in model.Workspace?.ItemLinks ?? []) {
            string Title(string key) => model.Workspace?.Items.FirstOrDefault(i => i.Id == link[key]?.GetValue<string>())?.Title ?? link[key]?.ToString() ?? "";
            evidence.Children.Add(Mute($"{Design.L("条目关系", "Item link")}: {Title("source_item_id")} · {link["relation"]} · {Title("target_item_id")}"));
        }
    }
    private string KindLabel(string kind) => ValueLabel("item", kind);
    private void RenderEditor()
    {
        editor.Children.Clear(); var draft = model.ItemDraft;
        Choice(editor, T("item_kind"), new[] { "section", "claim", "figure", "table", "methods", "supplement" }.Select(k => (k, KindLabel(k))), draft.Kind, s => model.ItemDraft = model.ItemDraft with { Kind = s });
        Input(editor, T("item_title"), draft.Title, s => model.ItemDraft = model.ItemDraft with { Title = s });
        Input(editor, T("description"), draft.Content, s => model.ItemDraft = model.ItemDraft with { Content = s }, true);
        Choice(editor, T("parent_item"), new[] { ("", T("no_parent")) }.Concat((model.Workspace?.Items ?? []).Where(i => i.Id != draft.Id).Select(i => (i.Id, i.Title))), draft.ParentId, s => model.ItemDraft = model.ItemDraft with { ParentId = s.Length == 0 ? null : s });
        Input(editor, Design.L("顺序", "Order"), draft.Ordinal.ToString(), s => model.ItemDraft = model.ItemDraft with { Ordinal = long.TryParse(s, out var n) ? n : -1 });
        editor.Children.Add(Button(T("save_item"), async () => { if (await model.SaveItemAsync()) { editing = false; RenderEvidence(); ShowTab(); } }));
        editor.Children.Add(Button(Design.L("返回证据列表", "Back to evidence"), () => { editing = false; ShowTab(); return Task.CompletedTask; }));
    }
    private FrameworkElement BindingCard(NativePublicationBinding b)
    {
        var body = new StackPanel { Spacing = 6 };
        var lineage = model.Workspace?.Lineage?.FirstOrDefault(v => v["binding_id"]?.GetValue<string>() == b.Id);
        body.Children.Add(Design.Text(lineage?["source_label"]?.GetValue<string>() ?? ValueLabel("source", b.SourceKind), 15));
        body.Children.Add(Mute(b.Purpose + "\n" + ValueLabel("selection", b.SelectionState) + " · " + ValueLabel("review", b.ReviewState) + " · " + ValueLabel("reproduction", b.ReproductionState)));
        if (lineage != null) body.Children.Add(Mute($"{T("lineage_quality")}: {lineage["quality"]} · {T("revision")}: {lineage["version_number"]}\n{T("source.run")}: {lineage["producing_run_title"]}\n{Design.L("校验", "Checksum")}: {lineage["checksum"]}"));
        if (model.Workspace?.Drift?.Any(d => d["binding_id"]?.GetValue<string>() == b.Id && d["has_drift"]?.GetValue<bool>() == true) == true) body.Children.Add(Warn(T("drift")));
        if (model.Workspace?.Revision?.State == "draft")
        {
            var selection = Choice(body, T("selection"), NativePublicationEvidence.SelectionChoices.Select(s => (s.Id, T("selection." + s.Id))), b.SelectionState, _ => { });
            var visible = Choice(body, T("visibility"), new[] { "private", "restricted", "public" }.Select(v => (v, T("visibility." + v))), b.Visibility, _ => { });
            body.Children.Add(Button(Design.L("保存证据状态", "Save evidence status"), async () => await model.MutateAsync(new() { ["action"] = "update_binding", ["binding_id"] = b.Id, ["selection_state"] = Selected(selection), ["visibility"] = Selected(visible) })));
        }
        else if (model.Workspace?.Revision?.State is "frozen" or "published")
        {
            var run = b.SourceKind == "run" ? b.SourceId : lineage?["producing_run_id"]?.GetValue<string>();
            if (run != null) body.Children.Add(Button(T("verify_run"), async () => await model.MutateAsync(new() { ["action"] = "verify", ["source_run_id"] = run, ["comparisons"] = new JsonArray() })));
        }
        var source = new TextBox { AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, IsReadOnly = true, MaxHeight = 220 }; source.Text = b.SourceSnapshotJson;
        body.Children.Add(Disclosure(T("exact_source"), source));
        foreach (var review in (model.Workspace?.Reviews ?? []).Where(r => r["binding_id"]?.GetValue<string>() == b.Id))
            body.Children.Add(Mute($"{T("review")}: {review["reviewer"]} · {review["method"]} · {review["result"]}\n{review["report_json"]}"));
        foreach (var supersession in (model.Workspace?.Supersessions ?? []).Where(r => r["old_binding_id"]?.GetValue<string>() == b.Id || r["new_binding_id"]?.GetValue<string>() == b.Id))
            body.Children.Add(Mute($"{T("supersession")}: {supersession["old_binding_id"]} → {supersession["new_binding_id"]}\n{supersession["reason"]}"));
        return Design.Card(body);
    }
    private void SetupSources()
    {
        sourcePanel.Children.Add(sourcePicker);
        Choice(sourcePicker, Design.L("来源类型", "Source type"), new[] { "files", "runs", "messages" }.Select(k => (k, T("sources." + k))), sourceKind, s => { sourceKind = s; sourceOffset = 0; _ = LoadSources(); });
        Input(sourcePicker, T("source_search"), "", s => sourceQuery = s);
        sourcePicker.Children.Add(Button(T("search"), async () => { sourceOffset = 0; await LoadSources(); }));
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
        SelectKind("message_span"); sourcePicker.Children.Add(Disclosure(T("add_precise"), form));
    }
    private Task LoadSources() => model.LoadSourcesAsync(sourceKind, sourceQuery, sourceOffset);
    private void RenderSourceRows()
    {
        sourceRows.Children.Clear(); sourcePaging.Children.Clear();
        if (model.SourcesError != null) sourceRows.Children.Add(Warn(model.SourcesError));
        if (model.SourcesLoading) sourceRows.Children.Add(Mute(T("loading")));
        foreach (var source in model.Sources.Sources) {
            var label = NativePublicationEvidence.SourceLabel(source);
            var choice = Button(label, () => { model.SelectSource(source); return Task.CompletedTask; });
            choice.Content = new TextBlock { Text = label, TextWrapping = TextWrapping.Wrap };
            choice.HorizontalAlignment = choice.HorizontalContentAlignment = HorizontalAlignment.Stretch;
            sourceRows.Children.Add(choice);
        }
        var prev = Button(T("previous"), async () => { sourceOffset = sourceOffset >= 50 ? sourceOffset - 50 : 0; await LoadSources(); }); prev.IsEnabled = sourceOffset > 0 && !model.SourcesLoading;
        var next = Button(T("next"), async () => { sourceOffset += 50; await LoadSources(); }); next.IsEnabled = model.Sources.HasMore && !model.SourcesLoading;
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
        var state = Choice(sourcePreview, T("selection"), NativePublicationEvidence.SelectionChoices.Select(s => (s.Id, T("selection." + s.Id))), "selected", _ => { });
        var visible = Choice(sourcePreview, T("visibility"), [("private", "私有"), ("restricted", "受限"), ("public", "公开")], "private", _ => { });
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
        if (model.PendingRun != null) { CancelBinding(); return; }
        if (confirmingFreeze) { InvalidateCheck(); return; }
        if (editing) { editing = false; ShowTab(); return; }
        if (tab == "sources" && model.Source != null) { model.ClearSource(); return; }
        if (tab != "evidence") { tab = "evidence"; ShowTab(); return; }
        base.HandleEscape();
    }
    public override void Dispose() { model.Changed -= Render; base.Dispose(); }
}
