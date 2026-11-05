using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed class NativeProjectExportPage : UserControl, IDisposable
{
    private readonly NativeProjectExportModel model;
    private readonly WispDesign design;
    private readonly Func<bool, Task<string?>> pickDestination;
    private readonly Action close;
    private readonly Grid layers = new();
    private Border? card;
    public bool IsModal => model.IsModal;
    public bool Busy => model.Busy;
    public bool Locks(string id) => model.LocksProject && model.ProjectId == id;
    public event Action? PresentationChanged;
    public NativeProjectExportPage(NativeProjectExportModel model, WispDesign design, Func<bool, Task<string?>> pickDestination, Action close)
    {
        this.model = model; this.design = design; this.pickDestination = pickDestination; this.close = close;
        HorizontalContentAlignment = HorizontalAlignment.Stretch; VerticalContentAlignment = VerticalAlignment.Stretch;
        Content = layers; model.Changed += Refresh; SizeChanged += (_, _) => Resize(); Refresh();
    }
    private TextBlock Text(string value, double size, string color = "text")
    { var text = design.Text(value, size); text.Foreground = design.Brush(color); text.LineHeight = design.FontSize(size * 1.55); text.LineStackingStrategy = LineStackingStrategy.BlockLineHeight; return text; }
    private string T(string key) => design.T(key);
    private void Refresh()
    {
        if (model.Closed) return;
        layers.Children.Clear(); card = null;
        if (model.Stage == NativeProjectExportStage.Cancelled) { PresentationChanged?.Invoke(); return; }
        var body = new StackPanel { Padding = new Thickness(IsModal ? 22 : 16), Spacing = IsModal ? 14 : 10 };
        var terminal = model.Stage is NativeProjectExportStage.Complete or NativeProjectExportStage.Failed;
        var titleKey = IsModal ? "projects.export_options_title" : model.Stage == NativeProjectExportStage.Complete ? "projects.transfer.export_complete" : model.Stage == NativeProjectExportStage.Failed ? "projects.transfer.export_failed" : "projects.transfer.export_title";
        var title = Text(T(titleKey), IsModal ? 17 : 14); title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        var head = new Grid(); head.ColumnDefinitions.Add(new()); head.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); head.Children.Add(title); body.Children.Add(head);
        if (terminal)
        {
            var dismiss = design.ToolButton(T("projects.transfer.done"), "close"); dismiss.Click += (_, _) => close(); Grid.SetColumn(dismiss, 1); head.Children.Add(dismiss);
        }
        if (IsModal)
        {
            var backdrop = new Grid(); NativeDialogVisuals.Backdrop(backdrop, design); layers.Children.Add(backdrop);
            body.Children.Add(Text(T("projects.export_zip_hint"), 13, "text-muted"));
            var choices = new StackPanel { Spacing = 10 };
            foreach (var (directory, label, help) in new[] { (true, "projects.export_directory", "projects.export_directory_hint"), (false, "projects.export_zip", "projects.export_archive_hint") })
            {
                var content = new StackPanel { Spacing = 4 };
                var strong = Text(T(label), 13.5); strong.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; content.Children.Add(strong);
                content.Children.Add(Text(T(help), 12.5, "text-muted"));
                var button = new Button { Content = content, Padding = new Thickness(13), HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch,
                    Background = design.Brush("bg-input"), BorderBrush = design.Brush("border-strong"), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(10) };
                button.Resources["ButtonBackgroundPointerOver"] = design.Brush("surface-hover"); button.Resources["ButtonBorderBrushPointerOver"] = design.Brush("clay");
                AutomationProperties.SetName(button, T(label)); AutomationProperties.SetHelpText(button, T(help));
                button.Click += async (_, _) => { await model.ExportAsync(directory, () => pickDestination(directory)); if (!model.Closed && model.Stage == NativeProjectExportStage.Cancelled) close(); };
                choices.Children.Add(button);
            }
            body.Children.Add(choices);
            var cancel = new Button { Content = T("settings.cancel"), HorizontalAlignment = HorizontalAlignment.Right }; design.ActionButton(cancel);
            cancel.Click += (_, _) => close(); body.Children.Add(cancel);
        }
        else if (!terminal)
        {
            body.Children.Add(Text(model.Stage == NativeProjectExportStage.Selecting ? T("projects.transfer.selecting_export_destination") : T("projects.transfer.preparing"), 12, "text-muted"));
            body.Children.Add(new ProgressBar { IsIndeterminate = true, Height = 7 });
        }
        if (!IsModal && model.Destination.Length > 0)
        {
            var path = Text(model.Destination, 11, "text-faint"); path.TextWrapping = TextWrapping.NoWrap; path.TextTrimming = TextTrimming.CharacterEllipsis; ToolTipService.SetToolTip(path, model.Destination); body.Children.Add(path);
        }
        if (model.Error is { } error)
        {
            const string prefix = "操作结果未能确认；不会自动重试。\n";
            body.Children.Add(Text(error.StartsWith(prefix, StringComparison.Ordinal) ? error[prefix.Length..] : error, 12, "text"));
        }
        card = design.Card(new ScrollViewer { Content = body, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch });
        card.Padding = new Thickness(0); card.CornerRadius = new CornerRadius(IsModal ? 16 : 12);
        card.HorizontalAlignment = IsModal ? HorizontalAlignment.Center : HorizontalAlignment.Right; card.VerticalAlignment = IsModal ? VerticalAlignment.Center : VerticalAlignment.Bottom;
        card.Margin = new Thickness(IsModal ? 16 : 24); card.Shadow = new ThemeShadow(); card.Translation = new System.Numerics.Vector3(0, 0, 24);
        AutomationProperties.SetName(card, T(titleKey)); AutomationProperties.SetAutomationId(card, IsModal ? "project-export-options" : "project-transfer-progress");
        AutomationProperties.SetLiveSetting(card, Microsoft.UI.Xaml.Automation.Peers.AutomationLiveSetting.Polite);
        layers.Children.Add(card); Resize(); PresentationChanged?.Invoke();
    }
    private void Resize()
    {
        if (card == null) return;
        var bounds = ProjectDialogLayout.ForViewport(ActualWidth, ActualHeight, IsModal ? 600 : 380);
        card.Width = Math.Min(bounds.Width, Math.Max(0, ActualWidth - (IsModal ? 32 : 48))); card.MaxHeight = bounds.MaxHeight;
    }
    public bool HandleEscape() { if (Busy) return false; close(); return true; }
    public void Dispose() { model.Changed -= Refresh; model.Dispose(); }
}
