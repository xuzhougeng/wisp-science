using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed class NativeProjectDeletePage : UserControl, IWorkspaceSheet
{
    private readonly NativeProjectDeleteModel model;
    private readonly WispDesign design;
    private readonly string workspaceDirectory;
    private readonly Action close;
    private readonly Grid overlay = new();
    private readonly DispatcherTimer timer = new() { Interval = TimeSpan.FromMilliseconds(100) };
    private Border? card;
    private Button? confirm;
    private bool disposed;
    public bool IsModal => true;
    public NativeProjectDeletePage(NativeProjectDeleteModel model, WispDesign design, string workspaceDirectory, Action close)
    {
        this.model = model; this.design = design; this.workspaceDirectory = workspaceDirectory; this.close = close;
        HorizontalContentAlignment = HorizontalAlignment.Stretch; VerticalContentAlignment = VerticalAlignment.Stretch;
        design.BindTypography(this); Content = overlay;
        NativeDialogVisuals.Backdrop(overlay, design);
        overlay.PointerPressed += (_, e) => { if (ReferenceEquals(e.OriginalSource, overlay)) HandleEscape(); };
        SizeChanged += (_, _) => Resize();
        timer.Tick += (_, _) => RefreshCountdown();
        Loaded += (_, _) => { timer.Start(); FocusSafeAction(); };
        Unloaded += (_, _) => timer.Stop();
        model.Changed += Render; Render();
    }
    private TextBlock Text(string value, double size, string color = "text")
    {
        var text = design.Text(value, size); text.Foreground = design.Brush(color);
        text.LineHeight = design.FontSize(size * 1.55); text.LineStackingStrategy = LineStackingStrategy.BlockLineHeight;
        return text;
    }
    private Button? safeAction;
    private Button Button(string key, Action action, bool primary = false, bool danger = false)
    {
        var label = design.Text(design.T(key), 13.5);
        label.Foreground = design.Brush(primary ? "on-clay" : danger ? "err" : "text");
        if (primary) label.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        var button = new Button { Content = label, Padding = new Thickness(16, 9, 16, 9), CornerRadius = new CornerRadius(10),
            Background = design.Brush(primary ? danger ? "err" : "clay" : "bg-input"),
            BorderBrush = design.Brush(danger ? "err" : primary ? "clay" : "border-strong"), BorderThickness = new Thickness(1) };
        button.Resources["ButtonBackgroundPointerOver"] = design.Brush(primary ? danger ? "err" : "clay-strong" : "surface-hover");
        button.Resources["ButtonBackgroundPressed"] = button.Resources["ButtonBackgroundPointerOver"];
        button.Resources["ButtonBorderBrushPointerOver"] = button.BorderBrush;
        button.Resources["ButtonBorderBrushPressed"] = button.BorderBrush;
        button.Resources["ButtonBackgroundDisabled"] = button.Background;
        button.Resources["ButtonBorderBrushDisabled"] = button.BorderBrush;
        button.Resources["ButtonForegroundDisabled"] = label.Foreground;
        AutomationProperties.SetName(button, design.T(key)); button.Click += (_, _) => action(); return button;
    }
    private void Render()
    {
        if (disposed) return;
        if (card != null) overlay.Children.Remove(card);
        confirm = null;
        var body = new StackPanel { Spacing = 14, Padding = new Thickness(22), TabFocusNavigation = KeyboardNavigationMode.Cycle };
        var title = design.Text(design.T(model.ConfirmingData ? "projects.delete_data_title" : "confirm.title"), 17);
        title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; body.Children.Add(title);
        var hint = Text(design.T(model.ConfirmingData ? "projects.delete_data_warning" : "projects.delete_confirm"), 13,
            model.ConfirmingData ? "err" : "text-muted");
        if (model.ConfirmingData) hint.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        body.Children.Add(hint);
        if (model.ConfirmingData)
        {
            var path = Text(design.T("projects.delete_data_path").Replace("{path}", workspaceDirectory), 12, "text-muted");
            path.FontFamily = design.CodeFont; path.TextWrapping = TextWrapping.Wrap; path.IsTextSelectionEnabled = true;
            body.Children.Add(new Border { Child = path, Padding = new Thickness(11, 9, 11, 9), Background = design.Brush("bg-input"),
                BorderBrush = design.Brush("border"), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(6) });
        }
        if (model.Error is { } error)
        {
            var notice = Text(design.L("操作结果未能确认。请关闭后检查项目列表；不会自动重试。", "The result could not be confirmed. Close to check the project list; this operation will not be retried automatically.") + "\n" + error, 13, "err");
            AutomationProperties.SetLiveSetting(notice, Microsoft.UI.Xaml.Automation.Peers.AutomationLiveSetting.Polite); body.Children.Add(notice);
        }
        if (model.Busy) body.Children.Add(new ProgressBar { IsIndeterminate = true, Height = 4 });
        var actions = new NativeActionWrap { RightAligned = true, Gap = 10, Margin = new Thickness(0, 4, 0, 0) };
        safeAction = Button(model.Attempted ? "caps.close" : model.ConfirmingData ? "settings.back" : "settings.cancel", HandleEscape);
        safeAction.IsEnabled = !model.Busy; actions.Children.Add(safeAction);
        if (!model.Attempted)
        {
            confirm = Button(model.ConfirmingData ? "projects.delete_data_confirm" : "projects.remove_only", async () =>
            {
                if (await model.DeleteAsync() && !disposed) close();
            }, primary: true, danger: model.ConfirmingData);
            actions.Children.Add(confirm);
            if (!model.ConfirmingData) actions.Children.Add(Button("projects.remove_with_data", model.ConfirmData, danger: true));
        }
        body.Children.Add(actions);
        card = design.Card(new ScrollViewer { Content = body, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch });
        card.Padding = new Thickness(0); card.CornerRadius = new CornerRadius(16); card.BorderBrush = design.Brush("border-strong");
        card.HorizontalAlignment = HorizontalAlignment.Center; card.VerticalAlignment = VerticalAlignment.Center;
        card.Shadow = new ThemeShadow(); card.Translation = new System.Numerics.Vector3(0, 0, 24);
        AutomationProperties.SetName(card, title.Text); AutomationProperties.SetAutomationId(card, model.ConfirmingData ? "project-delete-data" : "project-delete-choice");
        // A freshly rebuilt card cannot receive focus until it enters the visual
        // tree. Restore to Cancel/Back/Close after loading, never to deletion.
        var mounted = card;
        card.Loaded += (_, _) => { if (!disposed && ReferenceEquals(card, mounted)) FocusSafeAction(); };
        overlay.Children.Add(card); Resize(); RefreshCountdown(); design.ApplyTypography(this);
        if (IsLoaded) FocusSafeAction();
    }
    private void FocusSafeAction() => safeAction?.Focus(FocusState.Programmatic);
    private void RefreshCountdown()
    {
        if (disposed || confirm == null) return;
        var count = model.RemainingSeconds;
        var label = design.T(model.ConfirmingData ? count > 0 ? "projects.delete_data_countdown" : "projects.delete_data_confirm" : "projects.remove_only").Replace("{n}", count.ToString());
        if (((TextBlock)confirm.Content).Text != label)
        { ((TextBlock)confirm.Content).Text = label; AutomationProperties.SetName(confirm, label); }
        confirm.IsEnabled = model.CanDelete; confirm.Opacity = model.CanDelete ? 1 : .55;
    }
    private void Resize()
    {
        if (card == null) return;
        var bounds = ProjectDialogLayout.ForViewport(ActualWidth, ActualHeight, model.ConfirmingData ? 520 : 560);
        card.Width = bounds.Width; card.MaxHeight = bounds.MaxHeight;
    }
    public void HandleEscape()
    {
        if (disposed || model.Busy) return;
        if (model.ConfirmingData && !model.Attempted) model.Back(); else close();
    }
    public void Dispose() { disposed = true; timer.Stop(); model.Changed -= Render; model.Dispose(); }
}
