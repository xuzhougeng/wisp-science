using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

/// A retained run row: polling changes text/progress, never replaces the output viewer.
internal sealed class NativeRunMonitorCard : UserControl
{
    private readonly WispDesign design;
    private readonly TextBlock title, id, status, metadata, progressLabel, command, output, error;
    private readonly Border symbol, outputBox;
    private readonly ProgressBar progress = new() { Minimum = 0, Maximum = 100, Height = 3 };
    private readonly ScrollViewer outputScroll;
    private readonly Button cancel, harvest;
    private NativeRun run;
    private double? restoreOutput;
    public NativeRunMonitorCard(NativeRun initial, WispDesign design, Func<string, Task> details,
        Func<string, Task> cancelRun, Func<string, Task> harvestRun)
    {
        this.design = design; run = initial;
        var card = new StackPanel { Spacing = 8 };
        Content = new Border { Child = card, Padding = new(14, 12, 14, 12), CornerRadius = new(8), BorderThickness = new(1),
            Background = design.Brush("bg-elev"), BorderBrush = design.Brush("border-strong") };
        var heading = new Grid { ColumnSpacing = 9 };
        heading.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); heading.ColumnDefinitions.Add(new() { Width = new(1, GridUnitType.Star) });
        heading.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        symbol = new() { Width = 22, Height = 22, Background = design.Brush("clay-soft"), CornerRadius = new(6), VerticalAlignment = VerticalAlignment.Top };
        heading.Children.Add(symbol);
        var titles = new StackPanel { Spacing = 1 };
        title = design.Text("", 13); title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; title.MaxLines = 1; title.TextTrimming = TextTrimming.CharacterEllipsis;
        id = design.Text(initial.Id, 10.5); id.Foreground = design.Brush("text-faint"); id.FontFamily = design.Font(true); id.MaxLines = 1; id.TextTrimming = TextTrimming.CharacterEllipsis;
        titles.Children.Add(title); titles.Children.Add(id); Grid.SetColumn(titles, 1); heading.Children.Add(titles);
        status = design.Text("", 11.5);
        var badge = new Border { Child = status, MaxWidth = 110, Padding = new(6, 2, 6, 2), CornerRadius = new(6), Background = design.Brush("clay-soft"), VerticalAlignment = VerticalAlignment.Top };
        Grid.SetColumn(badge, 2); heading.Children.Add(badge); card.Children.Add(heading);
        metadata = design.Text("", 11.5); metadata.Foreground = design.Brush("text-muted"); metadata.FontFamily = design.Font(true); card.Children.Add(metadata);
        progressLabel = design.Text("", 12); card.Children.Add(progressLabel); card.Children.Add(progress);
        command = design.Text("", 11); command.FontFamily = design.Font(true); command.MaxLines = 1; command.TextTrimming = TextTrimming.CharacterEllipsis; card.Children.Add(command);
        output = design.Text("", 11); output.FontFamily = design.Font(true); output.IsTextSelectionEnabled = true;
        outputScroll = new() { Content = output, MaxHeight = 140, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled };
        outputScroll.LayoutUpdated += (_, _) =>
        {
            if (restoreOutput is not { } offset) return;
            restoreOutput = null; outputScroll.ChangeView(null, Math.Min(offset, outputScroll.ScrollableHeight), null, true);
        };
        outputBox = new() { Background = design.Brush("bg-sunken"), CornerRadius = new(6), Padding = new(8), Child = outputScroll }; card.Children.Add(outputBox);
        error = design.Text("", 12); error.Foreground = design.Brush("err"); card.Children.Add(error);
        var actions = new NativeActionWrap();
        var open = new Button { Content = design.L("运行详情", "Run details") }; design.QuietButton(open); open.Click += async (_, _) => await details(run.Id); actions.Children.Add(open);
        cancel = new Button(); design.QuietButton(cancel); cancel.Click += async (_, _) => await cancelRun(run.Id); actions.Children.Add(cancel);
        harvest = new Button { Content = design.L("收取", "Harvest") }; design.QuietButton(harvest); harvest.Click += async (_, _) => await harvestRun(run.Id); actions.Children.Add(harvest);
        card.Children.Add(actions);
    }
    public void Update(NativeRun value, bool readOnly, bool mutating)
    {
        var stateChanged = value.Status != run.Status || symbol.Child == null;
        run = value;
        title.Text = string.IsNullOrWhiteSpace(run.Title) ? run.Id : run.Title; ToolTipService.SetToolTip(title, title.Text);
        var key = "runs.status." + run.Status; var text = design.T(key); status.Text = text == key ? run.Status : text;
        var color = run.Status == "succeeded" ? "ok" : run.Status is "failed" or "lost" or "timed_out" ? "err" : "clay-strong";
        status.Foreground = design.Brush(color);
        if (stateChanged) symbol.Child = design.Icon(NativeSessionRunGroups.IsActive(run.Status) ? "activity-orbit" : run.Status == "succeeded" ? "check" : "circle-alert", 14, design.ColorText(color));
        var meter = NativeRunPresentation.Progress(run, design.Localization);
        progressLabel.Visibility = progress.Visibility = meter == null ? Visibility.Collapsed : Visibility.Visible;
        if (meter is { } p) { progressLabel.Text = p.Label; progress.Value = p.Percent ?? 0; progress.IsIndeterminate = p.Percent == null && NativeSessionRunGroups.IsActive(run.Status); }
        command.Text = run.Command ?? ""; command.Visibility = string.IsNullOrWhiteSpace(command.Text) ? Visibility.Collapsed : Visibility.Visible; ToolTipService.SetToolTip(command, command.Text);
        var nextOutput = NativeRunPresentation.Output(run);
        if (output.Text != nextOutput)
        {
            // Follow new output only while already at the bottom; otherwise
            // preserve the reader's position within this same viewer.
            restoreOutput = outputScroll.ScrollableHeight - outputScroll.VerticalOffset <= 8 ? double.MaxValue : outputScroll.VerticalOffset;
            output.Text = nextOutput;
        }
        outputBox.Visibility = nextOutput.Length == 0 ? Visibility.Collapsed : Visibility.Visible;
        error.Text = run.LastPollError ?? ""; error.Visibility = error.Text.Length == 0 ? Visibility.Collapsed : Visibility.Visible;
        cancel.Content = design.L(run.Status == "cancelling" ? "强制取消" : "取消运行", run.Status == "cancelling" ? "Force cancel" : "Cancel run");
        cancel.Visibility = !readOnly && run.Status is "running" or "submitted" or "cancelling" ? Visibility.Visible : Visibility.Collapsed;
        harvest.Visibility = !readOnly && run.HarvestedAt == null && run.CleanedAt == null && WorkspaceConversationModel.RunTerminal(run.Status) ? Visibility.Visible : Visibility.Collapsed;
        cancel.IsEnabled = harvest.IsEnabled = !mutating;
        UpdateClock(DateTimeOffset.UtcNow.ToUnixTimeSeconds(), run.LastPolledAt);
    }
    public void UpdateClock(long now, long? heartbeat)
    {
        var text = NativeRunPresentation.MonitorMetadata(run, now, heartbeat, design.Localization);
        if (metadata.Text != text) metadata.Text = text;
    }
}
