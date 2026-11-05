using System.Text.Json;
using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Input;
using Windows.ApplicationModel.DataTransfer;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class NativeConversationPage
{
    private ContentDialog? historyDialog;
    private bool historySubmitting;
    private ComboBox? memoryReplacement;
    private ComboBox? memoryScope;
    private NativeHistoryTarget? historyOwner;
    private readonly Button acknowledgeHistory = new() { Content = "已核对历史操作结果，允许继续" };

    public bool HandleHistoryEscape()
    {
        if (memoryReplacement?.IsDropDownOpen == true) { memoryReplacement.IsDropDownOpen = false; return true; }
        if (memoryScope?.IsDropDownOpen == true) { memoryScope.IsDropDownOpen = false; return true; }
        if (historyDialog != null) { if (!historySubmitting) historyDialog.Hide(); return true; }
        return false;
    }
    private void RefreshHistoryActions()
    {
        acknowledgeHistory.Visibility = model.HistoryUncertain ? Visibility.Visible : Visibility.Collapsed;
        acknowledgeHistory.Content = design.Localization.Choose("已核对历史操作结果，允许继续", "I have checked the history action result; continue");
        acknowledgeHistory.IsEnabled = !model.Busy && model.ConnectionError == null;
        if (historyOwner != null && (model.Snapshot?.SessionId != historyOwner.SessionId || model.Snapshot?.ProjectId != historyOwner.ProjectId))
        {
            historyDialog?.Hide();
        }
    }
    private Button TranscriptActionButton(NativeMessageAction action)
    {
        var label = design.T(action.LabelKey);
        var button = new Button();
        design.QuietButton(button);
        button.Padding = action.Icon == null ? new Thickness(8, 4, 8, 4) : new Thickness(4);
        button.MinWidth = 0; button.MinHeight = 0; button.CornerRadius = new CornerRadius(4);
        button.IsEnabled = action.Enabled;
        design.BindTypography(button, size: 11);
        button.Content = action.Icon == null ? label : design.Icon(action.Icon, 14, design.ColorText("text-faint"));
        button.Foreground = design.Brush("text-faint");
        AutomationProperties.SetName(button, label);
        AutomationProperties.SetAutomationId(button, "message-" + action.Kind);
        ToolTipService.SetToolTip(button, label);
        var inside = false;
        void Emphasis()
        {
            var emphasized = inside || button.FocusState != FocusState.Unfocused;
            button.Opacity = !button.IsEnabled ? (action.Icon == null ? .4 : .3) : action.Icon != null && !emphasized ? .6 : 1;
            if (action.Icon != null) button.Content = design.Icon(action.Icon, 14, design.ColorText(emphasized ? "text" : "text-faint"));
            else button.Foreground = design.Brush(emphasized ? "text" : "text-faint");
        }
        button.PointerEntered += (_, _) => { inside = true; Emphasis(); };
        button.PointerExited += (_, _) => { inside = false; Emphasis(); };
        button.GotFocus += (_, _) => Emphasis(); button.LostFocus += (_, _) => Emphasis();
        Emphasis();
        return button;
    }
    private void AddMessageActions(Panel actions, ConversationSnapshot page, int row, NativeMessageAction[] definitions)
    {
        var target = model.HistoryTarget(page, row);
        foreach (var action in definitions)
        {
            var button = TranscriptActionButton(action);
            if (action.Kind == "excerpt") ToolTipService.SetToolTip(button, design.T(model.ExcerptStart == null ? "msg.excerpt_start_hint" : "msg.excerpt_end_hint"));
            if (target != null) button.Tag = (target, action.Kind);
            button.Click += async (_, _) =>
            {
                if (disposed || model.CurrentProjectId != page.ProjectId || model.CurrentSessionId != page.SessionId) return;
                if (action.Kind == "copy")
                {
                    try { var package = new DataPackage(); package.SetText(page.Items[row].Text); Clipboard.SetContent(package); } catch { }
                }
                else if (action.Kind == "explore" && target != null && model.CanStartExploration(target)) startExploration?.Invoke(target);
                else if (action.Kind == "excerpt" && target != null && openExcerpt != null && model.CanExcerpt(target)) await SelectExcerpt(target);
                else if (target != null && model.CanHistoryAction(target, action.Kind))
                    await RunHistoryAction(model.CurrentHistoryTarget(target), action.Kind);
            };
            actions.Children.Add(button);
        }
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
                var result = await ShowHistoryDialog(target, design.T("msg.edit_confirm_title"), HistoryText(
                    design.T("msg.edit_confirm_hint")), design.T("msg.edit"));
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
                body.Children.Add(HistoryText(design.T("undo.body")));
                void Rows(string label, string[] values) { if (values.Length > 0) body.Children.Add(HistoryText(label + "\n" + string.Join("\n", values))); }
                Rows(design.T("undo.restore_files"), preview.RestoreFiles); Rows(design.T("undo.remove_files"), preview.RemoveFiles);
                Rows(design.T("undo.remove_artifacts"), preview.RemoveArtifacts); Rows(design.T("undo.unsupported_files"), preview.UnsupportedFiles);
                Rows(design.T("undo.conflicts"), preview.Conflicts);
                if (preview.RestoreFiles.Length == 0 && preview.RemoveFiles.Length == 0) body.Children.Add(HistoryText(design.T("undo.no_text_changes")));
                if (preview.UnsupportedFiles.Length > 0) body.Children.Add(HistoryText(design.T("undo.binary_warning")));
                var result = await ShowHistoryDialog(target, design.T("undo.title"), body, preview.Conflicts.Length == 0 ? design.T("undo.confirm") : null);
                if (result == ContentDialogResult.Primary)
                    await model.HistoryActionAsync(target, new() { ["kind"] = "undo" }, lifetime.Token);
            }
            else if (kind == "propose_memory")
            {
                var reply = await model.HistoryActionAsync(target, new() { ["kind"] = kind }, lifetime.Token);
                if (!reply.Success || disposed) return;
                if (reply.Result == null) { await ShowHistoryDialog(target, design.T("memory.proposal.title"), HistoryText(design.T("memory.proposal.none"))); return; }
                await EditTurnMemory(target, WorkspaceConversationModel.DecodeMemory(reply.Result, target));
            }
            else await model.HistoryActionAsync(target, new() { ["kind"] = kind }, lifetime.Token);
        }
        catch (Exception ex) when (!disposed)
        {
            await ShowHistoryDialog(target, design.Localization.Choose("历史操作", "Conversation history"), HistoryText(ex.Message));
        }
        finally
        {
            if (kind is "propose_memory" or "undo_preview" or "rewind") RestoreHistoryFocus(target, kind);
        }
    }
    private void RestoreHistoryFocus(NativeHistoryTarget target, string kind) => DispatcherQueue.TryEnqueue(
        Microsoft.UI.Dispatching.DispatcherQueuePriority.Low, () =>
        {
            if (disposed || model.CurrentProjectId != target.ProjectId || model.CurrentSessionId != target.SessionId) return;
            Button? Find(DependencyObject node)
            {
                if (node is Button { Tag: ValueTuple<NativeHistoryTarget, string> tag } button
                    && tag.Item1.Turn == target.Turn && tag.Item1.Role == target.Role && tag.Item2 == kind) return button;
                for (var i = 0; i < Microsoft.UI.Xaml.Media.VisualTreeHelper.GetChildrenCount(node); i++)
                    if (Find(Microsoft.UI.Xaml.Media.VisualTreeHelper.GetChild(node, i)) is { } found) return found;
                return null;
            }
            if (Find(transcript) is { IsEnabled: true } entry) entry.Focus(FocusState.Programmatic);
        });
    private TextBlock HistoryText(string text)
    {
        var block = design.Text(text, 14); block.IsTextSelectionEnabled = true; return block;
    }

    private async Task<ContentDialogResult> ShowHistoryDialog(NativeHistoryTarget target, string title, FrameworkElement content, string? primary = null,
        Func<ContentDialog, Task<bool>>? submit = null)
    {
        if (disposed || model.Snapshot?.SessionId != target.SessionId || model.Snapshot?.ProjectId != target.ProjectId || historyDialog != null || openingRunReview)
            return ContentDialogResult.None;
        var dialogRoot = XamlRoot;
        var scroller = new ScrollViewer { Content = content, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled,
            VerticalScrollBarVisibility = ScrollBarVisibility.Auto };
        var body = new StackPanel { Spacing = 20 };
        body.Children.Add(scroller);
        var footer = new NativeActionWrap { Gap = 10, RowGap = 8, RightAligned = true };
        body.Children.Add(footer);
        var dialog = new ContentDialog { Title = design.Text(title, 20), XamlRoot = dialogRoot, CornerRadius = new CornerRadius(16),
            RequestedTheme = design.Dark ? ElementTheme.Dark : ElementTheme.Light, Content = body };
        dialog.Resources["ContentDialogBackground"] = design.Brush("bg-elev");
        dialog.Resources["ContentDialogCornerRadius"] = new CornerRadius(16);
        dialog.Resources["ContentDialogMaxWidth"] = 620d;
        dialog.Resources["ContentDialogMinWidth"] = 0d;
        var result = ContentDialogResult.None;
        var cancel = new Button { Content = primary == null ? design.Localization.Choose("关闭", "Close") : design.T("settings.cancel") };
        design.ActionButton(cancel); cancel.Click += (_, _) => { if (!historySubmitting) dialog.Hide(); }; footer.Children.Add(cancel);
        var confirm = new Button { Content = primary ?? "" };
        design.ActionButton(confirm, true);
        confirm.Resources["ButtonBackgroundDisabled"] = confirm.Resources["ButtonBorderBrushDisabled"] = design.Brush("clay");
        confirm.Resources["ButtonForegroundDisabled"] = design.Brush("on-clay");
        if (primary != null) footer.Children.Add(confirm);
        historyDialog = dialog; historyOwner = target;
        var previousFocus = FocusManager.GetFocusedElement(XamlRoot) as Control;
        design.BindTypography(dialog);
        dialog.Closing += (_, args) => args.Cancel = historySubmitting;
        void Fit()
        {
            scroller.MaxHeight = Math.Max(80, Math.Min(520, dialogRoot.Size.Height - 240));
            dialog.Resources["ContentDialogMaxWidth"] = Math.Max(0, Math.Min(620, dialogRoot.Size.Width - 32));
        }
        void RootChanged(XamlRoot _, XamlRootChangedEventArgs __) => Fit();
        Fit(); dialogRoot.Changed += RootChanged;
        confirm.Click += async (_, _) =>
        {
            if (historySubmitting) return;
            historySubmitting = true; confirm.IsEnabled = cancel.IsEnabled = scroller.IsEnabled = false;
            confirm.Opacity = .5;
            if (submit != null && confirm.Content is TextBlock pendingLabel) pendingLabel.Text = design.T("memory.proposal.saving");
            try { if (submit == null || await submit(dialog)) result = ContentDialogResult.Primary; }
            finally
            {
                historySubmitting = false; cancel.IsEnabled = scroller.IsEnabled = true;
                confirm.IsEnabled = !model.HistoryUncertain;
                confirm.Opacity = confirm.IsEnabled ? 1 : .5;
                if (confirm.Content is TextBlock label) label.Text = primary ?? "";
                if (result == ContentDialogResult.Primary || model.CurrentProjectId != target.ProjectId || model.CurrentSessionId != target.SessionId)
                    dialog.Hide();
            }
        };
        try { await dialog.ShowAsync(); return result; }
        finally
        {
            dialogRoot.Changed -= RootChanged;
            if (historyDialog == dialog) historyDialog = null;
            if (!disposed && previousFocus?.IsLoaded == true) previousFocus.Focus(FocusState.Programmatic);
        }
    }
    private async Task EditTurnMemory(NativeHistoryTarget target, NativeTurnMemoryProposal proposal)
    {
        var content = new TextBox { AcceptsReturn = true, Text = proposal.Content, TextWrapping = TextWrapping.Wrap, MinHeight = 210,
            Padding = new Thickness(12, 11, 12, 11), CornerRadius = new CornerRadius(8), Background = design.Brush("bg-input"),
            BorderBrush = design.Brush("border-strong"), BorderThickness = new Thickness(1), Foreground = design.Brush("text") };
        var scope = new ComboBox { HorizontalAlignment = HorizontalAlignment.Stretch,
            ItemsSource = new[] { design.T("memory.proposal.scope_project"), design.T("memory.proposal.scope_global") }, SelectedIndex = proposal.Scope == "global" ? 1 : 0 };
        var replacement = new ComboBox { HorizontalAlignment = HorizontalAlignment.Stretch };
        design.BindTypography(content, size: 13);
        foreach (var select in new[] { scope, replacement })
        {
            design.BindTypography(select, size: 13); select.MinHeight = 38; select.Padding = new Thickness(11, 7, 11, 7);
            select.CornerRadius = new CornerRadius(8); select.Background = design.Brush("bg-input");
            select.BorderBrush = design.Brush("border-strong"); select.BorderThickness = new Thickness(1); select.Foreground = design.Brush("text");
        }
        AutomationProperties.SetName(content, design.T("memory.proposal.content"));
        AutomationProperties.SetName(scope, design.T("memory.proposal.scope"));
        AutomationProperties.SetName(replacement, design.T("memory.proposal.replace"));
        TextBlock Hint(string text)
        {
            var block = design.Text(text, 13); block.Foreground = design.Brush("text-muted"); design.BindLineHeight(block, 13, 1.55); return block;
        }
        StackPanel Field(string key, UIElement control)
        {
            var field = new StackPanel { Spacing = 7 }; field.Children.Add(Hint(design.T(key))); field.Children.Add(control); return field;
        }
        replacement.Items.Add(design.T("memory.proposal.add_new"));
        foreach (var memory in proposal.GlobalMemories) replacement.Items.Add(memory.Content.Length > 90 ? memory.Content[..90] + "…" : memory.Content);
        replacement.SelectedIndex = 0;
        var replaceField = Field("memory.proposal.replace", replacement);
        replaceField.Children.Add(Hint(design.T("memory.proposal.replace_hint")));
        replaceField.Visibility = scope.SelectedIndex == 1 && proposal.GlobalMemories.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
        scope.SelectionChanged += (_, _) => replaceField.Visibility = scope.SelectedIndex == 1 && proposal.GlobalMemories.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
        var body = new StackPanel { Spacing = 12 };
        var error = HistoryText(""); error.Foreground = design.Brush("err"); error.Visibility = Visibility.Collapsed;
        void ShowError(string message)
        {
            error.Text = message; error.Visibility = Visibility.Visible;
            DispatcherQueue.TryEnqueue(Microsoft.UI.Dispatching.DispatcherQueuePriority.Low, () =>
            {
                if (disposed || !error.IsLoaded) return;
                error.UpdateLayout();
                error.StartBringIntoView(new BringIntoViewOptions { VerticalAlignmentRatio = 1, AnimationDesired = false });
            });
        }
        if (proposal.Trigger == "tool_failures") body.Children.Add(HistoryText(design.Localization.Format("memory.proposal.failure_hint",
            ("total", proposal.ToolCalls.ToString()), ("failed", proposal.FailedToolCalls.ToString()),
            ("rate", proposal.FailureRate.ToString("0.0", System.Globalization.CultureInfo.InvariantCulture)))));
        else body.Children.Add(Hint(design.T(proposal.Trigger == "explicit" ? "memory.proposal.explicit_hint" : "memory.proposal.manual_hint")));
        body.Children.Add(Field("memory.proposal.scope", scope)); body.Children.Add(replaceField);
        body.Children.Add(Field("memory.proposal.content", NativeDialogVisuals.Input(content, design))); body.Children.Add(error);
        memoryReplacement = replacement; memoryScope = scope;
        try
        {
            await ShowHistoryDialog(target, design.T(proposal.Trigger == "tool_failures" ? "memory.proposal.failure_title" : "memory.proposal.title"), body, design.T("memory.proposal.confirm"), async dialog =>
            {
                if (string.IsNullOrWhiteSpace(content.Text) || System.Text.Encoding.UTF8.GetByteCount(content.Text) > 16_384)
                { ShowError(string.IsNullOrWhiteSpace(content.Text) ? design.T("memory.proposal.empty")
                    : design.Localization.Choose("记忆内容不得超过 16 KB。", "Memory must not exceed 16 KB.")); return false; }
                var saved = await model.HistoryActionAsync(target, new() { ["kind"] = "confirm_memory", ["content"] = content.Text,
                    ["scope"] = scope.SelectedIndex == 1 ? "global" : "project",
                    ["replace_id"] = scope.SelectedIndex == 1 && replacement.SelectedIndex > 0 ? proposal.GlobalMemories[replacement.SelectedIndex - 1].Id : null }, lifetime.Token);
                if (!saved.Success)
                {
                    ShowError(model.HistoryFailureReason is { } reason
                        ? design.Localization.Choose("保存结果未能确认；请关闭并核对会话，不要重复保存。", "The save could not be confirmed. Close and check the conversation before trying again.") + "\n" + reason
                        : model.OperationError ?? design.Localization.Choose("会话状态已变化，请关闭后重新提取记忆。", "The conversation changed. Close this dialog and prepare the memory again."));
                }
                return saved.Success;
            });
        }
        finally { memoryReplacement = null; memoryScope = null; }
    }
}
