using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed partial class NativeSettingsSectionPage
{
    private NativeProjectSyncJoinModel? syncJoin;
    private Grid? syncJoinOverlay;
    private Button? syncJoinOwner, syncJoinSubmit, syncJoinCancel;
    private TextBlock? syncJoinError;

    private void RefreshSyncJoin()
    {
        if (syncJoin == null || model.Closed) return;
        if (syncJoin.ModalOpen && syncJoinOverlay == null) ShowSyncJoin();
        if (!syncJoin.ModalOpen && syncJoinOverlay != null)
        {
            HideSyncJoin();
            DispatcherQueue.TryEnqueue(Microsoft.UI.Dispatching.DispatcherQueuePriority.Low, () =>
            {
                if (!model.Closed && syncJoin.ModalOpen == false && syncJoinOwner is { IsLoaded: true, IsEnabled: true } owner)
                    owner.Focus(FocusState.Programmatic);
            });
        }
        if (syncJoinSubmit != null) { syncJoinSubmit.IsEnabled = syncJoin.CanJoin; syncJoinSubmit.Opacity = syncJoin.CanJoin ? 1 : .55; }
        if (syncJoinCancel != null) syncJoinCancel.IsEnabled = !syncJoin.Busy;
        if (syncJoinError != null)
        {
            syncJoinError.Text = syncJoin.DisplayError ?? "";
            syncJoinError.Visibility = syncJoinError.Text.Length == 0 ? Visibility.Collapsed : Visibility.Visible;
        }
        LockConfirmation();
    }
    private void HideSyncJoin()
    {
        if (syncJoinOverlay != null) ModalChanged?.Invoke(null);
        syncJoinOverlay = null; syncJoinSubmit = null; syncJoinCancel = null; syncJoinError = null;
    }
    private void ShowSyncJoin()
    {
        var state = syncJoin!;
        var overlay = new Grid();
        NativeDialogVisuals.Backdrop(overlay, Design, syncJoin: true);
        syncJoinOverlay = overlay;
        AutomationProperties.SetName(overlay, T("projects.sync.join_title"));
        var body = new StackPanel { Padding = new Thickness(30, 28, 30, 24) };
        var header = new Grid { ColumnSpacing = 16 };
        header.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        header.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var title = Design.Text(T("projects.sync.join_title"), 20); title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        title.VerticalAlignment = VerticalAlignment.Center; header.Children.Add(title);
        var close = Design.ToolButton(T("projects.cancel"), "close"); close.Width = close.Height = 34;
        close.Margin = new Thickness(0, -5, -7, -5); close.Click += (_, _) => state.Dismiss();
        Grid.SetColumn(close, 1); header.Children.Add(close); body.Children.Add(header);
        var hint = Design.Text(T("projects.sync.join_hint"), 13); hint.Foreground = Design.Brush("text-muted");
        hint.LineHeight = 21.45 * Design.Typography.UiSize / 14; hint.Margin = new Thickness(0, 14, 0, 22); body.Children.Add(hint);

        var codeHead = new Grid { ColumnSpacing = 12, Margin = new Thickness(0, 0, 0, 8) };
        codeHead.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        codeHead.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var label = Design.Text(T("projects.sync.code_label"), 12); label.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        label.VerticalAlignment = VerticalAlignment.Center; codeHead.Children.Add(label);
        var guideContent = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 5 };
        guideContent.Children.Add(Design.Icon("doc", 14, Design.ColorText("clay-strong")));
        var guideText = Design.Text(T("projects.sync.guide"), 12); guideText.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        guideText.Foreground = Design.Brush("clay-strong"); guideContent.Children.Add(guideText);
        var guide = new Button { Content = guideContent, Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent),
            BorderThickness = new Thickness(0), Padding = new Thickness(0, 3, 0, 3), MinHeight = 0, MinWidth = 0 };
        AutomationProperties.SetName(guide, T("projects.sync.guide"));
        guide.Click += async (_, _) => await Windows.System.Launcher.LaunchUriAsync(new Uri("https://github.com/xuzhougeng/wisp-science/blob/main/docs/" + (Design.Localization.Locale == "zh" ? "project-sync.zh-CN.md" : "project-sync.md")));
        Grid.SetColumn(guide, 1); codeHead.Children.Add(guide); body.Children.Add(codeHead);

        var input = new TextBox { AcceptsReturn = true, Text = state.Code, TextWrapping = TextWrapping.Wrap,
            PlaceholderText = T("projects.sync.code_placeholder"), IsSpellCheckEnabled = false, IsTextPredictionEnabled = false,
            Height = 128, Padding = new Thickness(13, 12, 13, 12), CornerRadius = new CornerRadius(8), BorderThickness = new Thickness(1),
            Background = Design.Brush("bg-input"), Foreground = Design.Brush("text"), BorderBrush = Design.Brush("border-strong") };
        Design.BindTypography(input, code: true, size: 13); AutomationProperties.SetLabeledBy(input, label);
        AutomationProperties.SetHelpText(input, hint.Text); textMenus.Register(input);
        input.TextChanged += (_, _) => state.SetCode(input.Text);
        ScrollViewer.SetVerticalScrollBarVisibility(input, ScrollBarVisibility.Auto); body.Children.Add(NativeDialogVisuals.Input(input, Design));
        syncJoinError = Warn(""); syncJoinError.Margin = new Thickness(0, 8, 0, 0);
        AutomationProperties.SetLiveSetting(syncJoinError, Microsoft.UI.Xaml.Automation.Peers.AutomationLiveSetting.Assertive);
        body.Children.Add(syncJoinError);
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 10, HorizontalAlignment = HorizontalAlignment.Right, Margin = new Thickness(0, 22, 0, 0) };
        var cancel = ChannelButton(T("projects.cancel"), () => { state.Dismiss(); return Task.CompletedTask; });
        var submit = ChannelButton(T("projects.sync.join_action"), async () =>
        {
            if (await state.JoinAsync() && !model.Closed && state.JoinedProjectId is { } id) ProjectRequested?.Invoke(id);
        }, primary: true);
        syncJoinCancel = cancel; syncJoinSubmit = submit;
        submit.Padding = new Thickness(18, 8, 18, 8);
        if (submit.Content is TextBlock submitLabel) submitLabel.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        submit.MinWidth = 180; submit.Resources["ButtonBackgroundDisabled"] = Design.Brush("clay");
        submit.Resources["ButtonForegroundDisabled"] = new SolidColorBrush(Microsoft.UI.Colors.White);
        submit.Resources["ButtonBorderBrushDisabled"] = Design.Brush("clay");
        cancel.Padding = new Thickness(18, 8, 18, 8); cancel.MinHeight = submit.MinHeight = 40;
        actions.Children.Add(cancel); actions.Children.Add(submit); body.Children.Add(actions);
        var scroll = new ScrollViewer { Content = body, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        var card = new Border { Child = scroll, Background = Design.Brush("bg-elev"), CornerRadius = new CornerRadius(16),
            Shadow = new ThemeShadow(), Translation = new System.Numerics.Vector3(0, 0, 24),
            HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center, TabFocusNavigation = KeyboardNavigationMode.Cycle };
        overlay.Children.Add(card);
        void Resize()
        {
            var compact = overlay.ActualWidth <= 520;
            card.Width = Math.Min(580, Math.Max(0, overlay.ActualWidth - (compact ? 24 : 32)));
            card.MaxHeight = Math.Max(0, overlay.ActualHeight - (compact ? 24 : 32)); body.Width = card.Width;
            body.Padding = compact ? new Thickness(20, 24, 20, 20) : new Thickness(30, 28, 30, 24);
            card.VerticalAlignment = compact ? VerticalAlignment.Bottom : VerticalAlignment.Center;
            card.Margin = new Thickness(0, 0, 0, compact ? 12 : 0);
            actions.Orientation = compact ? Orientation.Vertical : Orientation.Horizontal;
            actions.HorizontalAlignment = compact ? HorizontalAlignment.Stretch : HorizontalAlignment.Right;
            if (compact != (actions.Children[0] == submit))
            {
                actions.Children.Clear(); actions.Children.Add(compact ? submit : cancel); actions.Children.Add(compact ? cancel : submit);
            }
            foreach (var button in actions.Children.OfType<Button>()) button.HorizontalAlignment = compact ? HorizontalAlignment.Stretch : HorizontalAlignment.Left;
        }
        overlay.SizeChanged += (_, _) => Resize();
        overlay.Loaded += (_, _) =>
        {
            Resize();
            DispatcherQueue.TryEnqueue(Microsoft.UI.Dispatching.DispatcherQueuePriority.Low, () =>
            { if (ReferenceEquals(syncJoinOverlay, overlay) && input.IsLoaded) input.Focus(FocusState.Programmatic); });
        };
        // The reference does not dismiss when clicking its backdrop.
        ModalChanged?.Invoke(overlay); Design.ApplyTypography(body);
    }
}
