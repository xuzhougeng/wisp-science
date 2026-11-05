using System.Text.Json;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class NativeConversationPage
{
    private readonly StackPanel queueRows = new() { Spacing = 6 };
    private readonly ScrollViewer queueScroll = new() { MaxHeight = 160, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled,
        HorizontalContentAlignment = HorizontalAlignment.Stretch, VerticalScrollBarVisibility = ScrollBarVisibility.Auto };
    private readonly Border queueCard = new() { Visibility = Visibility.Collapsed, Padding = new(8), CornerRadius = new(16), BorderThickness = new(1),
        MaxWidth = PreviewLayout.ConversationMaxWidth, Margin = new(16, 0, 16, 8) };
    private readonly StackPanel queueHeader = new() { Orientation = Orientation.Horizontal, Spacing = 7, Padding = new(6, 2, 6, 7) };
    private Flyout? queueMenu;
    private NativeQueueTarget? queueOwner;
    private string? queueFingerprint;
    private readonly Func<NativeQueueTarget, string, bool>? sendQueuedSideChat;
    private readonly Func<bool>? canSendQueuedSideChat;
    public bool HandleQueueEscape()
    {
        if (queueMenu == null) return false;
        queueMenu.Hide(); return true;
    }
    private void InitializeQueue()
    {
        queueScroll.Content = queueRows;
        var body = new StackPanel(); body.Children.Add(queueHeader); body.Children.Add(queueScroll); queueCard.Child = body;
        AutomationProperties.SetAutomationId(queueCard, "composer-queue");
        SizeChanged += (_, _) => queueScroll.MaxHeight = ActualHeight < 650 ? 100 : 180;
    }
    private SolidColorBrush MixBrush(string first, string second, double weight)
    {
        var a = design.Brush(first).Color; var b = design.Brush(second).Color;
        var mix = NativeQueuePresentation.Mix((a.A, a.R, a.G, a.B), (b.A, b.R, b.G, b.B), weight);
        return new(Windows.UI.Color.FromArgb(mix.A, mix.R, mix.G, mix.B));
    }
    private void RenderQueue()
    {
        if (queueOwner != null && (!model.CanQueueAction(queueOwner, "cancel") || model.ShowingHistory)) queueMenu?.Hide();
        var snapshot = model.Snapshot;
        var rows = NativeQueuePresentation.Rows(snapshot, model.QueuedFollowUp);
        var recoveries = model.QueueRecoveries;
        var failed = snapshot?.Queue?.Outcomes.Where(outcome => outcome.State == "failed" && !recoveries.Any(row => row.Id == outcome.Id)).TakeLast(3).ToArray() ?? [];
        var fingerprint = JsonSerializer.Serialize(rows) + JsonSerializer.Serialize(recoveries) + JsonSerializer.Serialize(failed)
            + $"/{snapshot?.ProjectId}/{snapshot?.SessionId}/{snapshot?.Running}/{snapshot?.Stopping}/{snapshot?.ReadOnly}/{snapshot?.Queue?.CanCutIn}/{model.Busy}/{model.QueueUncertain}/{model.UncertainSend}/{model.ConnectionError}/{model.ShowingHistory}/{model.CanRecoverQueue}/{canSendQueuedSideChat?.Invoke()}"
            + $"/{design.Dark}/{design.LightPalette}/{design.DarkPalette}/{design.Typography}/{design.Localization.Locale}";
        if (queueFingerprint == fingerprint) return;
        queueFingerprint = fingerprint;
        queueCard.Visibility = !model.ShowingHistory && (rows.Length > 0 || recoveries.Length > 0 || failed.Length > 0) ? Visibility.Visible : Visibility.Collapsed;
        queueCard.Background = MixBrush("bg-elev", "clay-soft", .82); queueCard.BorderBrush = MixBrush("clay", "border", .20);
        AutomationProperties.SetName(queueCard, design.T("queue.region"));
        queueHeader.Children.Clear(); if (rows.Length > 0) queueHeader.Children.Add(NativeLoadingVisuals.QueueDot(design));
        var heading = design.Text(rows.Length > 0 ? design.Localization.Format("queue.header", ("n", rows.Length.ToString(System.Globalization.CultureInfo.InvariantCulture)))
            : recoveries.Length > 0 ? design.L("待恢复草稿", "Drafts to restore") : design.L("队列状态", "Queue status"), 11);
        heading.Foreground = design.Brush("clay-strong"); heading.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; queueHeader.Children.Add(heading);
        queueRows.Children.Clear();
        foreach (var item in rows)
        {
            var row = new Grid { ColumnSpacing = 10, RowSpacing = 6 };
            row.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); row.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            row.RowDefinitions.Add(new() { Height = GridLength.Auto }); row.RowDefinitions.Add(new() { Height = GridLength.Auto });
            var text = design.Text(item.Text, 13.5); text.MaxLines = 4; text.TextTrimming = TextTrimming.CharacterEllipsis; text.Margin = new(0, 2, 0, 2);
            ToolTipService.SetToolTip(text, item.Text); row.Children.Add(text);
            FrameworkElement? trailing = null;
            if (item.State == "cutin_pending")
            {
                var state = design.Text(design.T("queue.waiting_current_step"), 11); state.Foreground = design.Brush("text-muted"); trailing = state;
                AutomationProperties.SetLiveSetting(state, Microsoft.UI.Xaml.Automation.Peers.AutomationLiveSetting.Polite);
            }
            else if (item.Item is { } queued && model.QueueTarget(queued) is { } target)
            {
                var actions = new NativeActionWrap { Gap = 2, RowGap = 2, VerticallyCentered = true };
                Button Action(string kind, string icon, string key)
                {
                    var button = design.ToolButton(design.T(key), icon); button.Content = design.Icon(icon, 15);
                    button.Width = button.Height = 28; button.MinWidth = button.MinHeight = 0; button.Padding = new(5);
                    button.IsEnabled = model.CanQueueAction(target, kind); button.Opacity = button.IsEnabled ? .72 : .3; actions.Children.Add(button); return button;
                }
                if (snapshot?.Queue?.CanCutIn == true)
                {
                    var guide = Action("cut_in", "bolt", "queue.cut_in"); guide.Width = double.NaN; guide.Padding = new(10, 0, 10, 0); guide.CornerRadius = new(999);
                    guide.Opacity = guide.IsEnabled ? 1 : .3;
                    var label = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 5 }; label.Children.Add(design.Icon("bolt", 14, design.ColorText("clay-strong")));
                    var guideText = design.Text(design.T("queue.cut_in"), 11.5); guideText.IsTextSelectionEnabled = false; guideText.Foreground = design.Brush("clay-strong"); label.Children.Add(guideText);
                    guide.Content = label; guide.Background = design.Brush("clay-soft"); guide.Click += async (_, _) => await model.QueueActionAsync(target, new() { ["kind"] = "cut_in" }, lifetime.Token);
                }
                var more = Action("cancel", "more", "queue.more"); more.Click += (_, _) => ShowQueueMenu(more, target);
                if (rows.Length > 1) foreach (var (kind, icon, key) in new[] { ("move_up", "up", "queue.move_up"), ("move_down", "chevron-down", "queue.move_down") })
                { var button = Action(kind, icon, key); button.Click += async (_, _) => await model.QueueActionAsync(target, new() { ["kind"] = kind }, lifetime.Token); }
                var edit = Action("cancel", "edit", "queue.edit"); edit.Click += async (_, _) => {
                    if (await model.TakeQueuedDraftAsync(target, lifetime.Token)) { model.RestoreQueueDraft(target.Item.Id); composer.Focus(FocusState.Programmatic); }
                };
                var remove = Action("cancel", "trash", "queue.remove"); remove.Click += async (_, _) => await model.QueueActionAsync(target, new() { ["kind"] = "cancel" }, lifetime.Token);
                trailing = actions;
            }
            if (trailing != null)
            {
                trailing.VerticalAlignment = VerticalAlignment.Center; Grid.SetColumn(trailing, 1); row.Children.Add(trailing);
                row.SizeChanged += (_, e) => { var stacked = e.NewSize.Width < 620; Grid.SetRow(trailing, stacked ? 1 : 0); Grid.SetColumn(trailing, stacked ? 0 : 1); Grid.SetColumnSpan(trailing, stacked ? 2 : 1); Grid.SetColumnSpan(text, stacked ? 2 : 1); };
            }
            queueRows.Children.Add(new Border { Child = row, Padding = new(10, 8, 8, 8), CornerRadius = new(8), BorderThickness = new(1), BorderBrush = design.Brush("border"), Background = design.Brush("bg-elev") });
        }
        foreach (var recovery in recoveries)
        {
            var preview = design.Text(recovery.Text, 13.5); preview.MaxLines = 4; preview.TextTrimming = TextTrimming.CharacterEllipsis;
            ToolTipService.SetToolTip(preview, recovery.Text); queueRows.Children.Add(preview);
            queueRows.Children.Add(design.Text(NativeQueuePresentation.RecoveryReason(recovery.Reason, design.Localization), 12));
            var restore = new Button { Content = design.L("恢复排队草稿", "Restore queued draft"), IsEnabled = model.CanRecoverQueue, HorizontalAlignment = HorizontalAlignment.Left };
            design.ActionButton(restore); restore.Click += (_, _) => { model.RestoreQueueDraft(recovery.Id); composer.Focus(FocusState.Programmatic); }; queueRows.Children.Add(restore);
        }
        foreach (var outcome in failed) queueRows.Children.Add(design.Text(design.L($"排队消息 {outcome.Id} 执行失败，请检查会话。", $"Queued message {outcome.Id} failed. Check the conversation."), 12));
    }
    private void ShowQueueMenu(Button owner, NativeQueueTarget target)
    {
        queueMenu?.Hide();
        var menu = new Flyout { Placement = FlyoutPlacementMode.TopEdgeAlignedRight }; queueMenu = menu; queueOwner = target;
        var body = new StackPanel { Spacing = 2 };
        void Entry(string icon, string key, bool enabled, Func<Task> action)
        {
            var button = new Button { HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Left, IsEnabled = enabled };
            button.Opacity = enabled ? 1 : .3;
            design.QuietButton(button); button.Padding = new(10, 8, 10, 8);
            var content = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 }; content.Children.Add(design.Icon(icon, 16));
            var label = design.Text(design.T(key), 13); label.IsTextSelectionEnabled = false; content.Children.Add(label); button.Content = content;
            AutomationProperties.SetName(button, design.T(key)); button.Click += async (_, _) => { menu.Hide(); await action(); }; body.Children.Add(button);
        }
        Entry("sync", "composer.interrupt_replace", model.CanQueueAction(target, "replace"), async () => { await model.QueueActionAsync(target, new() { ["kind"] = "replace" }, lifetime.Token); });
        Entry("chat", "composer.side_chat", sendQueuedSideChat != null && canSendQueuedSideChat?.Invoke() == true && model.CanQueueAction(target, "cancel"), async () => {
            if (canSendQueuedSideChat?.Invoke() != true || !await model.TakeQueuedDraftAsync(target, lifetime.Token)) return;
            if (!disposed && sendQueuedSideChat?.Invoke(target, target.Item.Message) == true) model.ConsumeReturnedQueueDraft(target);
        });
        menu.Content = body;
        var presenter = new Style(typeof(FlyoutPresenter)); presenter.Setters.Add(new Setter(Control.PaddingProperty, new Thickness(6))); presenter.Setters.Add(new Setter(Control.BackgroundProperty, design.Brush("bg-elev"))); menu.FlyoutPresenterStyle = presenter;
        menu.Closed += (_, _) => { if (queueMenu == menu) queueMenu = null; if (!disposed && model.Snapshot?.SessionId == target.SessionId) owner.Focus(FocusState.Programmatic); };
        menu.ShowAt(owner);
    }
}
