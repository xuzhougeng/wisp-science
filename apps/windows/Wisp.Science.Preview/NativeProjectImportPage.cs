using System.Globalization;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

/// Import choices/recovery are modal; transfer status stays in the bottom corner.
internal sealed class NativeProjectImportPage : UserControl, IDisposable
{
    private readonly NativeProjectImportModel model;
    private readonly WispDesign design;
    private readonly Func<Task<string?>> pickFolder, pickArchive;
    private readonly Func<string, bool, Task> completed;
    private readonly Action close;
    private readonly Action<FlyoutBase> register;
    private readonly Grid layers = new();
    private NativeProjectImportStage? rendered;
    private string? renderedProgress;
    private Border? card;
    private Button? recover, cancel, dismiss;
    private TextBox? name;
    private TextBlock? error;
    private ProgressBar? progress;
    public bool IsModal => model.IsModal;
    public bool Busy => model.Busy;
    public event Action? PresentationChanged;

    public NativeProjectImportPage(NativeProjectImportModel model, WispDesign design, Func<Task<string?>> pickFolder,
        Func<Task<string?>> pickArchive, Func<string, bool, Task> completed, Action close, Action<FlyoutBase> register)
    {
        this.model = model; this.design = design; this.pickFolder = pickFolder; this.pickArchive = pickArchive;
        this.completed = completed; this.close = close; this.register = register;
        HorizontalContentAlignment = HorizontalAlignment.Stretch; VerticalContentAlignment = VerticalAlignment.Stretch;
        Content = layers; model.Changed += Refresh; SizeChanged += (_, _) => Resize(); Refresh();
    }
    private TextBlock Text(string text, double size, string color = "text")
    { var block = design.Text(text, size); block.FontWeight = Microsoft.UI.Text.FontWeights.Normal; block.Foreground = design.Brush(color); return block; }
    private string T(string key) => design.T(key);
    private Button Action(string label, Func<Task> action, bool primary = false)
    {
        var button = new Button { Content = Text(label, 13.5), Padding = new Thickness(16, 9, 16, 9), CornerRadius = new CornerRadius(10),
            Background = design.Brush(primary ? "clay" : "bg-input"), BorderBrush = design.Brush(primary ? "clay" : "border-strong"), BorderThickness = new Thickness(1) };
        if (primary && button.Content is TextBlock caption) { caption.Foreground = new SolidColorBrush(Microsoft.UI.Colors.White); caption.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; }
        AutomationProperties.SetName(button, label); button.Click += async (_, _) => await action(); return button;
    }
    private void Refresh()
    {
        if (model.Closed) return;
        if (rendered != model.Stage || !model.IsModal && renderedProgress != model.ProgressKey)
        {
            rendered = model.Stage; renderedProgress = model.ProgressKey; layers.Children.Clear(); card = null; recover = cancel = dismiss = null; name = null; error = null; progress = null;
            if (model.IsModal) Modal(); else if (model.Stage != NativeProjectImportStage.Cancelled) Transfer();
            Resize(); PresentationChanged?.Invoke();
        }
        if (name != null) name.IsReadOnly = model.Busy;
        if (recover != null) { recover.IsEnabled = model.CanRecover; recover.Opacity = recover.IsEnabled ? 1 : .55; ((TextBlock)recover.Content).Text = T(model.Busy ? "projects.recovering" : "projects.recover_action"); }
        if (cancel != null) cancel.IsEnabled = !model.Busy;
        if (dismiss != null) dismiss.IsEnabled = !model.Busy;
        if (error != null)
        {
            const string prefix = "操作结果未能确认；不会自动重试。\n";
            var message = model.Error ?? ""; if (message.StartsWith(prefix, StringComparison.Ordinal)) message = message[prefix.Length..];
            error.Text = message; error.Visibility = message.Length == 0 ? Visibility.Collapsed : Visibility.Visible;
        }
        if (progress != null) progress.Visibility = model.Busy ? Visibility.Visible : Visibility.Collapsed;
    }
    private void Modal()
    {
        var overlay = new Grid(); NativeDialogVisuals.Backdrop(overlay, design); layers.Children.Add(overlay);
        var body = new StackPanel { Padding = new Thickness(22), Spacing = 14 };
        var recovery = model.Stage == NativeProjectImportStage.Preview;
        var title = Text(T(recovery ? "projects.recover_title" : "projects.import"), 17); title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        AutomationProperties.SetName(overlay, title.Text); AutomationProperties.SetAutomationId(overlay, recovery ? "workspace-session-recovery" : "project-import-options");
        if (recovery)
        {
            var header = new Grid(); header.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); header.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            title.VerticalAlignment = VerticalAlignment.Center; header.Children.Add(title);
            dismiss = design.ToolButton(T("projects.cancel"), "close"); dismiss.Click += (_, _) => HandleEscape(); Grid.SetColumn(dismiss, 1); header.Children.Add(dismiss); body.Children.Add(header);
            Recovery(body);
        }
        else
        {
            body.Children.Add(title);
            var hint = Text(T("projects.import_options_hint"), 13, "text-muted"); hint.LineHeight = design.FontSize(19.5); body.Children.Add(hint);
            var choices = new StackPanel { Spacing = 10 };
            foreach (var (kind, key) in new[] { (NativeProjectImportKind.Directory, "import_in_place"), (NativeProjectImportKind.Archive, "import_zip"), (NativeProjectImportKind.Recovery, "recover_workspace") })
            {
                var content = new StackPanel { Spacing = 4 };
                var strong = Text(T("projects." + key), 13.5); strong.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; content.Children.Add(strong);
                var help = Text(T("projects." + key + "_hint"), 12.5, "text-muted"); help.LineHeight = design.FontSize(18.75); content.Children.Add(help);
                var button = new Button { Content = content, Padding = new Thickness(13), HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch,
                    Background = design.Brush("bg-input"), BorderBrush = design.Brush("border-strong"), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(10) };
                button.Resources["ButtonBackgroundPointerOver"] = design.Brush("surface-hover"); button.Resources["ButtonBorderBrushPointerOver"] = design.Brush("clay");
                AutomationProperties.SetName(button, strong.Text); AutomationProperties.SetHelpText(button, help.Text);
                button.Click += async (_, _) => await Choose(kind); choices.Children.Add(button);
            }
            body.Children.Add(choices);
        }
        var actions = new NativeActionWrap { RightAligned = true, Gap = 10, Margin = new Thickness(0, 4, 0, 0) };
        cancel = Action(T("settings.cancel"), () => { HandleEscape(); return Task.CompletedTask; }); actions.Children.Add(cancel);
        if (recovery)
        {
            recover = Action(T("projects.recover_action"), async () => { if (await model.RecoverAsync() && !model.Closed && model.ImportedProjectId is { } id) await completed(id, true); }, true);
            actions.Children.Add(recover);
        }
        body.Children.Add(actions);
        card = Surface(new ScrollViewer { Content = body, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch }, 16);
        card.TabFocusNavigation = KeyboardNavigationMode.Cycle; card.HorizontalAlignment = HorizontalAlignment.Center; card.VerticalAlignment = VerticalAlignment.Center;
        overlay.Children.Add(card);
        card.Loaded += (_, _) => { if (name != null) { name.Focus(FocusState.Programmatic); name.SelectAll(); } else cancel?.Focus(FocusState.Programmatic); };
    }
    private void Recovery(StackPanel body)
    {
        var notice = Text(T("projects.recover_notice"), 12.5, "text-muted"); notice.LineHeight = design.FontSize(18.75);
        var tint = new Grid(); tint.Children.Add(new Border { Background = design.Brush("clay"), Opacity = .08, CornerRadius = new CornerRadius(6) });
        tint.Children.Add(new Border { Child = notice, Padding = new Thickness(11, 9, 11, 9) }); body.Children.Add(tint);
        var group = new StackPanel { Spacing = 5 }; var label = Text(T("proj_settings.name"), 12, "text-muted"); group.Children.Add(label);
        name = new TextBox { Text = model.Name, MinHeight = 36, Padding = new Thickness(11, 9, 11, 9), CornerRadius = new CornerRadius(10), Background = design.Brush("bg-input"), Foreground = design.Brush("text"), BorderBrush = design.Brush("border-strong"), BorderThickness = new Thickness(1) };
        design.BindTypography(name, size: 14); AutomationProperties.SetLabeledBy(name, label);
        name.TextChanged += (_, _) => { model.Name = name.Text; Refresh(); }; var textMenu = new TextCommandBarFlyout(); name.ContextFlyout = textMenu; register(textMenu);
        group.Children.Add(NativeDialogVisuals.Input(name, design)); body.Children.Add(group);
        var path = Text(model.SourcePath, 11.5, "text-faint"); path.FontFamily = design.Font(code: true); path.TextWrapping = TextWrapping.NoWrap; path.TextTrimming = TextTrimming.CharacterEllipsis; ToolTipService.SetToolTip(path, model.SourcePath);
        body.Children.Add(new Border { Child = path, Padding = new Thickness(10, 8, 10, 8), Background = design.Brush("bg-sunken"), BorderBrush = design.Brush("border"), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(6) });
        var stats = new Grid { ColumnSpacing = 8 };
        var metrics = new[] { ("recoverable_session_count", "recover_sessions"), ("message_count", "recover_messages"), ("valid_archive_count", "recover_archives") };
        for (var i = 0; i < metrics.Length; i++)
        {
            stats.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); var tile = new StackPanel { Spacing = 2 };
            var count = Text(model.Count(metrics[i].Item1).ToString(CultureInfo.InvariantCulture), 18); count.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; tile.Children.Add(count); tile.Children.Add(Text(T("projects." + metrics[i].Item2), 11.5, "text-muted"));
            var frame = new Border { Child = tile, Padding = new Thickness(10), CornerRadius = new CornerRadius(6), Background = design.Brush("bg-sunken"), BorderBrush = design.Brush("border"), BorderThickness = new Thickness(1) }; Grid.SetColumn(frame, i); stats.Children.Add(frame);
        }
        body.Children.Add(stats);
        var start = model.Timestamp("earliest_message_at"); var end = model.Timestamp("latest_message_at");
        string Date(long seconds) => DateTimeOffset.FromUnixTimeSeconds(seconds).ToLocalTime().ToString("g", CultureInfo.GetCultureInfo(design.Localization.Locale == "zh" ? "zh-CN" : "en-US"));
        body.Children.Add(Text(start.HasValue && end.HasValue ? design.Localization.Format("projects.recover_range", ("start", Date(start.Value)), ("end", Date(end.Value))) : T("projects.recover_range_unknown"), 12, "text-muted"));
        if (model.Count("invalid_archive_count") > 0 || model.Count("duplicate_archive_count") > 0)
            body.Children.Add(Text(design.Localization.Format("projects.recover_skipped", ("invalid", model.Count("invalid_archive_count").ToString()), ("duplicate", model.Count("duplicate_archive_count").ToString())), 12, "warn"));
        if (model.Count("recoverable_session_count") == 0) body.Children.Add(Text(T("projects.recover_empty"), 12, "err"));
        error = Text("", 12, "err"); AutomationProperties.SetLiveSetting(error, Microsoft.UI.Xaml.Automation.Peers.AutomationLiveSetting.Assertive); body.Children.Add(error);
    }
    private void Transfer()
    {
        var body = new StackPanel { Padding = new Thickness(15, 14, 15, 14), Spacing = 9 };
        var terminal = model.Stage is NativeProjectImportStage.Complete or NativeProjectImportStage.Failed;
        var titleKey = model.Stage == NativeProjectImportStage.Complete ? "projects.transfer.import_complete" : model.Stage == NativeProjectImportStage.Failed ? "projects.transfer.import_failed" : "projects.transfer.import_title";
        var header = new Grid(); header.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); header.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var title = Text(T(titleKey), 14); title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; header.Children.Add(title);
        if (terminal) { dismiss = design.ToolButton(T("projects.transfer.done"), "close"); dismiss.Click += (_, _) => close(); Grid.SetColumn(dismiss, 1); header.Children.Add(dismiss); }
        body.Children.Add(header);
        if (!terminal)
        {
            body.Children.Add(Text(T(model.ProgressKey), 12, "text-muted")); progress = new ProgressBar { IsIndeterminate = true, Height = 7 }; body.Children.Add(progress);
            if (model.ProgressKey == "projects.transfer.selecting_import_destination") body.Children.Add(Text(T("projects.transfer.import_destination_hint"), 12, "text-muted"));
        }
        if (model.SourcePath.Length > 0) { var path = Text(model.SourcePath, 11, "text-faint"); path.TextWrapping = TextWrapping.NoWrap; path.TextTrimming = TextTrimming.CharacterEllipsis; ToolTipService.SetToolTip(path, model.SourcePath); body.Children.Add(path); }
        error = Text("", 12, "err"); body.Children.Add(error);
        card = Surface(body, 10); card.HorizontalAlignment = HorizontalAlignment.Right; card.VerticalAlignment = VerticalAlignment.Bottom; card.Margin = new Thickness(20);
        AutomationProperties.SetLiveSetting(card, Microsoft.UI.Xaml.Automation.Peers.AutomationLiveSetting.Polite); layers.Children.Add(card);
    }
    private Border Surface(UIElement child, double radius) => new() { Child = child, Background = design.Brush("bg-elev"), BorderBrush = design.Brush("border-strong"), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(radius), Shadow = new ThemeShadow(), Translation = new System.Numerics.Vector3(0, 0, 24) };
    private void Resize()
    {
        if (card == null) return;
        var bounds = ProjectDialogLayout.ForViewport(ActualWidth, ActualHeight, model.IsModal ? model.Stage == NativeProjectImportStage.Preview ? 600 : 560 : 380);
        card.Width = bounds.Width; card.MaxHeight = bounds.MaxHeight;
    }
    private async Task Choose(NativeProjectImportKind kind)
    {
        await model.ChooseAsync(kind, kind == NativeProjectImportKind.Archive ? pickArchive : pickFolder, pickFolder);
        if (model.Closed) return;
        if (model.Stage == NativeProjectImportStage.Cancelled) close();
        else if (model.Stage == NativeProjectImportStage.Complete && model.ImportedProjectId is { } id) await completed(id, kind != NativeProjectImportKind.Archive);
    }
    public bool HandleEscape()
    {
        if (model.Busy) return model.IsModal;
        close(); return true;
    }
    public void Dispose() { model.Changed -= Refresh; model.Dispose(); }
}
