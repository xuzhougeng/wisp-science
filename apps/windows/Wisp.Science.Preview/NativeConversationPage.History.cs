using System.Text.Json;
using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class NativeConversationPage
{
    private MenuFlyout? historyMenu;
    private ContentDialog? historyDialog;
    private ComboBox? memoryReplacement;
    private NativeHistoryTarget? historyOwner;
    private readonly Button acknowledgeHistory = new() { Content = "已核对历史操作结果，允许继续" };

    public bool HandleHistoryEscape()
    {
        if (memoryReplacement?.IsDropDownOpen == true) { memoryReplacement.IsDropDownOpen = false; return true; }
        if (historyMenu != null) { historyMenu.Hide(); return true; }
        if (historyDialog != null) { historyDialog.Hide(); return true; }
        return false;
    }
    private void RefreshHistoryActions()
    {
        acknowledgeHistory.Visibility = model.HistoryUncertain ? Visibility.Visible : Visibility.Collapsed;
        acknowledgeHistory.IsEnabled = !model.Busy && model.ConnectionError == null;
        if (historyOwner != null && (model.Snapshot?.SessionId != historyOwner.SessionId || model.Snapshot?.ProjectId != historyOwner.ProjectId))
        {
            historyMenu?.Hide(); historyDialog?.Hide();
        }
    }
    private void AddHistoryActions(StackPanel actions, ConversationSnapshot page, int row)
    {
        if (model.HistoryTarget(page, row) is not { } target) return;
        var more = MessageAction("更多");
        more.Click += (_, _) =>
        {
            historyMenu?.Hide();
            var menu = new MenuFlyout(); historyMenu = menu; historyOwner = target;
            void Add(string label, string kind)
            {
                var entry = new MenuFlyoutItem { Text = label, IsEnabled = model.CanHistoryAction(target, kind) };
                entry.Click += async (_, _) => await RunHistoryAction(model.CurrentHistoryTarget(target), kind);
                menu.Items.Add(entry);
            }
            Add("从这里创建分支", "branch");
            if (target.Role == "user") Add("回退并编辑这条消息…", "rewind");
            else
            {
                Add("审查整个会话", "review");
                Add("提取本轮记忆…", "propose_memory");
                Add("撤销本轮…", "undo_preview");
            }
            menu.Closed += (_, _) => { if (historyMenu == menu) historyMenu = null; };
            menu.ShowAt(more);
        };
        actions.Children.Add(more);
    }
    private async Task RunHistoryAction(NativeHistoryTarget target, string kind)
    {
        try
        {
            if (kind == "branch")
            {
                var reply = await model.HistoryActionAsync(target, new() { ["kind"] = kind,
                    ["checkpoint"] = target.Role == "user" ? "before_user" : "after_response" }, lifetime.Token);
                if (reply.Success && reply.Result?.GetValue<string>() is { Length: > 0 } id && openHistoryBranch != null)
                    await openHistoryBranch(target.ProjectId, target.SessionId, id);
            }
            else if (kind == "rewind")
            {
                var result = await ShowHistoryDialog(target, "回退并编辑", HistoryText(
                    "将移除这条消息及后续对话。工作区文件不会恢复。若需保留对话，可取消后选择“从这里创建分支”。"), "确认回退");
                if (result == ContentDialogResult.Primary)
                    await model.HistoryActionAsync(target, new() { ["kind"] = "rewind" }, lifetime.Token);
            }
            else if (kind == "undo_preview")
            {
                var reply = await model.HistoryActionAsync(target, new() { ["kind"] = kind }, lifetime.Token);
                if (!reply.Success || reply.Result == null || disposed) return;
                var preview = reply.Result.Deserialize<NativeTurnUndoPreview>(ConversationSnapshot.JsonOptions)
                    ?? throw new InvalidDataException("Missing undo preview");
                if (preview.RestoreFiles == null || preview.RemoveFiles == null || preview.RemoveArtifacts == null
                    || preview.UnsupportedFiles == null || preview.Conflicts == null) throw new InvalidDataException("Incomplete undo preview");
                var body = new StackPanel { Spacing = 12 };
                body.Children.Add(HistoryText("撤销最新一轮，并按下列预览恢复文件和产物。外部操作及不支持恢复的文件不会撤销。"));
                void Rows(string label, string[] values) { if (values.Length > 0) body.Children.Add(HistoryText(label + "\n" + string.Join("\n", values))); }
                Rows("恢复文件", preview.RestoreFiles); Rows("移除文件", preview.RemoveFiles); Rows("移除产物", preview.RemoveArtifacts);
                Rows("无法恢复", preview.UnsupportedFiles); Rows("文件冲突（请先处理）", preview.Conflicts);
                var result = await ShowHistoryDialog(target, "撤销本轮", body, preview.Conflicts.Length == 0 ? "确认撤销" : null);
                if (result == ContentDialogResult.Primary)
                    await model.HistoryActionAsync(target, new() { ["kind"] = "undo" }, lifetime.Token);
            }
            else if (kind == "propose_memory")
            {
                var reply = await model.HistoryActionAsync(target, new() { ["kind"] = kind }, lifetime.Token);
                if (!reply.Success || disposed) return;
                if (reply.Result == null) { await ShowHistoryDialog(target, "本轮记忆", HistoryText("本轮没有可保存的记忆建议。")); return; }
                await EditTurnMemory(target, WorkspaceConversationModel.DecodeMemory(reply.Result, target));
            }
            else await model.HistoryActionAsync(target, new() { ["kind"] = kind }, lifetime.Token);
        }
        catch (Exception ex) when (!disposed)
        {
            await ShowHistoryDialog(target, "历史操作", HistoryText(ex.Message));
        }
    }
    private static TextBlock HistoryText(string text) => new() { Text = text, TextWrapping = TextWrapping.Wrap, IsTextSelectionEnabled = true };

    private async Task<ContentDialogResult> ShowHistoryDialog(NativeHistoryTarget target, string title, FrameworkElement content, string? primary = null,
        Func<ContentDialog, Task<bool>>? submit = null)
    {
        if (disposed || model.Snapshot?.SessionId != target.SessionId || model.Snapshot?.ProjectId != target.ProjectId || historyDialog != null || queueDialog != null || openingRunReview)
            return ContentDialogResult.None;
        var dialog = new ContentDialog { Title = title, XamlRoot = XamlRoot, PrimaryButtonText = primary ?? "",
            CloseButtonText = primary == null ? "关闭" : "取消", DefaultButton = ContentDialogButton.Close,
            RequestedTheme = design.Dark ? ElementTheme.Dark : ElementTheme.Light,
            Content = new ScrollViewer { MaxHeight = 380, Content = content, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled } };
        historyDialog = dialog; historyOwner = target;
        if (submit != null) dialog.PrimaryButtonClick += async (_, args) =>
        {
            var deferral = args.GetDeferral();
            try { args.Cancel = !await submit(dialog); }
            finally { deferral.Complete(); }
        };
        try { return await dialog.ShowAsync(); }
        finally { if (historyDialog == dialog) historyDialog = null; }
    }
    private async Task EditTurnMemory(NativeHistoryTarget target, NativeTurnMemoryProposal proposal)
    {
        var content = new TextBox { Text = proposal.Content, AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, MinHeight = 150 };
        var scope = new RadioButtons { Header = "保存范围", ItemsSource = new[] { "项目记忆", "全局记忆" }, SelectedIndex = proposal.Scope == "global" ? 1 : 0 };
        var replacement = new ComboBox { Header = "全局记忆", HorizontalAlignment = HorizontalAlignment.Stretch, MaxWidth = 420 };
        replacement.Items.Add("新增记忆");
        foreach (var memory in proposal.GlobalMemories) replacement.Items.Add(memory.Content.Length > 90 ? memory.Content[..90] + "…" : memory.Content);
        replacement.SelectedIndex = 0;
        replacement.Visibility = scope.SelectedIndex == 1 ? Visibility.Visible : Visibility.Collapsed;
        scope.SelectionChanged += (_, _) => replacement.Visibility = scope.SelectedIndex == 1 ? Visibility.Visible : Visibility.Collapsed;
        var body = new StackPanel { Spacing = 12 };
        var error = HistoryText(""); error.Foreground = design.Brush("clay-strong");
        body.Children.Add(HistoryText($"第 {target.Turn.UserIndex + 1} 轮 · 工具调用 {proposal.ToolCalls} 次 · 失败 {proposal.FailedToolCalls} 次"));
        body.Children.Add(content); body.Children.Add(scope); body.Children.Add(replacement); body.Children.Add(error);
        memoryReplacement = replacement;
        try
        {
            await ShowHistoryDialog(target, "确认本轮记忆", body, "保存记忆", async dialog =>
            {
                if (string.IsNullOrWhiteSpace(content.Text) || System.Text.Encoding.UTF8.GetByteCount(content.Text) > 16_384)
                { error.Text = "请填写记忆内容，且内容不得超过 16 KB。"; return false; }
                var saved = await model.HistoryActionAsync(target, new() { ["kind"] = "confirm_memory", ["content"] = content.Text,
                    ["scope"] = scope.SelectedIndex == 1 ? "global" : "project",
                    ["replace_id"] = scope.SelectedIndex == 1 && replacement.SelectedIndex > 0 ? proposal.GlobalMemories[replacement.SelectedIndex - 1].Id : null }, lifetime.Token);
                if (!saved.Success)
                {
                    error.Text = model.OperationError ?? "会话状态已变化，请关闭后重新提取记忆。";
                    dialog.IsPrimaryButtonEnabled = !model.HistoryUncertain;
                }
                return saved.Success;
            });
        }
        finally { memoryReplacement = null; }
    }
    private bool RenderHistoryReview(StackPanel card, ConversationItem item)
    {
        if (item.Role != "review") return false;
        try
        {
            var report = JsonNode.Parse(item.Text)!;
            card.Children.Add(HistoryText("会话审查 · " + (report["summary"]?.GetValue<string>() ?? "")));
            if (report["review_status"]?.GetValue<string>() is { Length: > 0 } reviewStatus)
                card.Children.Add(HistoryText("审查状态：" + reviewStatus));
            if (report["coverage_gaps"] is JsonArray gaps && gaps.Count > 0)
                card.Children.Add(HistoryText("证据缺口\n" + string.Join("\n", gaps.Select(gap => gap?.GetValue<string>()))));
            if (report["findings"] is JsonArray findings) foreach (var finding in findings.OfType<JsonObject>())
                card.Children.Add(HistoryText($"{finding["severity"]} · {finding["verdict"]}\n{finding["claim"]}\n证据：{finding["evidence"]}\n建议：{finding["fix"]}"));
            return true;
        }
        catch (JsonException) { return false; }
    }
}
