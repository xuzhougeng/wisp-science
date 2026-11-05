using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class NativeWorkspacePanel
{
    private bool completedRunsOpen;
    private readonly Microsoft.UI.Dispatching.DispatcherQueueTimer runsTimer;
    private readonly Dictionary<string, NativeRunMonitorCard> runCards = [];
    private StackPanel? activeRows, completedRows, completedGroup, healthBanner;
    private TextBlock? activeCount, completedCount, healthText;
    private Border? activeEmpty;
    private Button? completedToggle;
    private Image? completedArrow;
    private bool listWasEmpty;
    private int runTick;
    private bool ShowingRuns => model.Tabs.Selected == "runs" && runReview?.Visible != true;
    private string RefreshWarning => model.RunRefreshError is { } error
        ? design.Localization.Format("runs.refresh_failed", ("error", error))
        : model.RunReadDelayed(DateTimeOffset.UtcNow) ? design.T("runs.refresh_delayed") : "";
    private void StartRunsPolling(object sender, RoutedEventArgs e)
    {
        if (!disposed && model.CanPollRuns) { runTick = 0; runsTimer.Start(); }
    }
    private void StopRunsPolling(object sender, RoutedEventArgs e) => runsTimer.Stop();
    private async void TickRuns(Microsoft.UI.Dispatching.DispatcherQueueTimer sender, object args)
    {
        if (disposed || !IsLoaded) return;
        if (ShowingRuns)
        {
            UpdateRunWarning();
            var now = DateTimeOffset.UtcNow.ToUnixTimeSeconds();
            foreach (var run in model.SessionRuns.Active)
                if (run.Status is "submitted" or "running" or "cancelling" && runCards.TryGetValue(run.Id, out var card) && card.IsLoaded)
                    card.UpdateClock(now, run.LastPolledAt);
        }
        runTick = (runTick + 1) % 5;
        if (runTick == 0 || conversation?.Snapshot?.Running == true || model.SessionRuns.Active.Any(r =>
            r.Status is "submitted" or "running" or "cancelling" && !string.IsNullOrEmpty(r.ProgressJson) && r.ProgressJson != "{}"))
            await RefreshRunListAsync();
    }
    private async Task RefreshRunListAsync()
    {
        if (disposed || !IsLoaded) return;
        var changed = await model.PollRunsAsync(lifetime.Token);
        if (disposed || !IsLoaded) return;
        if (changed) tabStrip.Refresh();
        if (!ShowingRuns) return;
        var groups = model.SessionRuns;
        if (listWasEmpty != groups.Empty) { Render(); return; }
        UpdateRunWarning();
        if (changed && !groups.Empty) UpdateRunRows(groups);
    }
    private void UpdateRunWarning()
    {
        if (healthText == null || healthBanner == null) return;
        var warning = RefreshWarning;
        if (healthText.Text == warning) return;
        if (listWasEmpty) { Render(); return; }
        healthText.Text = warning;
        healthBanner.Visibility = warning.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
    }
    private readonly ContentControl runsEmpty = new() { Visibility = Visibility.Collapsed,
        HorizontalContentAlignment = HorizontalAlignment.Center, VerticalContentAlignment = VerticalAlignment.Center };

    private void RenderRuns()
    {
        RenderRunDetail();
        var groups = model.SessionRuns;
        listWasEmpty = groups.Empty;
        runCards.Clear();
        var warning = RefreshWarning;
        healthBanner = new StackPanel { Spacing = 6, Visibility = warning.Length == 0 ? Visibility.Collapsed : Visibility.Visible };
        healthText = design.Text(warning, 12); healthText.Foreground = design.Brush("warn"); healthBanner.Children.Add(healthText);
        var retry = new Button { Content = design.T("runs.refresh") };
        design.ActionButton(retry); retry.Click += async (_, _) => await RefreshRunListAsync();
        healthBanner.Children.Add(retry); body.Children.Add(healthBanner);
        if (groups.Empty)
        {
            if (model.Loading || warning.Length > 0 || model.Error != null || model.SelectedRunId != null) return;
            var empty = new StackPanel { Spacing = 8, Padding = new Thickness(12, 24, 12, 24), HorizontalAlignment = HorizontalAlignment.Center };
            empty.Children.Add(new Microsoft.UI.Xaml.Shapes.Rectangle { Width = 28, Height = 28, RadiusX = 6, RadiusY = 6,
                Stroke = design.Brush("border-strong"), StrokeThickness = 1.5, StrokeDashArray = new DoubleCollection { 2, 2 },
                Fill = design.Brush("bg-sunken"), HorizontalAlignment = HorizontalAlignment.Center });
            var title = design.Text(design.T("right.no_runs.title"), 13); title.TextAlignment = TextAlignment.Center;
            title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; title.Foreground = design.Brush("text-muted"); empty.Children.Add(title);
            var hint = design.Text(design.T("right.no_runs.body"), 12); hint.TextAlignment = TextAlignment.Center;
            hint.Foreground = design.Brush("text-faint"); hint.MaxWidth = 260; design.BindLineHeight(hint, 12, 1.5); empty.Children.Add(hint);
            runsEmpty.Content = empty; runsEmpty.Visibility = Visibility.Visible; scroll.Visibility = Visibility.Collapsed; return;
        }
        var active = new StackPanel { Spacing = 8 };
        var head = new Grid { ColumnSpacing = 6 };
        head.ColumnDefinitions.Add(new() { Width = new(1, GridUnitType.Star) });
        head.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); head.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var label = RunGroupLabel(design.T("right.runs_active")); label.VerticalAlignment = VerticalAlignment.Center; head.Children.Add(label);
        var count = design.Text(groups.Active.Length.ToString(), 12); count.Foreground = design.Brush("text-faint"); count.VerticalAlignment = VerticalAlignment.Center;
        activeCount = count; Grid.SetColumn(count, 1); head.Children.Add(count);
        var refresh = new Button { Content = design.Icon("sync", 15), IsEnabled = !model.Loading };
        design.QuietButton(refresh); refresh.Width = refresh.Height = 24; refresh.MinWidth = refresh.MinHeight = 0; refresh.Padding = new(3);
        AutomationProperties.SetName(refresh, design.T("runs.refresh")); ToolTipService.SetToolTip(refresh, design.T("runs.refresh"));
        refresh.Click += async (_, _) => await RefreshRunListAsync(); Grid.SetColumn(refresh, 2); head.Children.Add(refresh);
        active.Children.Add(head);
        var none = design.Text(design.T("right.runs_active_empty"), 12.5); none.Foreground = design.Brush("text-faint");
        activeEmpty = new Border { Child = none, Padding = new(12), CornerRadius = new(8), BorderBrush = design.Brush("border-strong"), BorderThickness = new(1) };
        active.Children.Add(activeEmpty); activeRows = new StackPanel { Spacing = 8 }; active.Children.Add(activeRows); body.Children.Add(active);
        completedGroup = new StackPanel { Spacing = 8 };
        var titleRow = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4 };
        completedArrow = design.Icon("chevron-right", 14); titleRow.Children.Add(completedArrow);
        completedCount = RunGroupLabel(""); titleRow.Children.Add(completedCount);
        completedToggle = new Button { Content = titleRow, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Left };
        design.QuietButton(completedToggle); completedToggle.Padding = new(0); completedToggle.MinHeight = 24;
        completedToggle.Click += (_, _) => { completedRunsOpen = !completedRunsOpen; UpdateRunRows(model.SessionRuns); };
        completedGroup.Children.Add(completedToggle);
        completedRows = new StackPanel { Spacing = 8 }; completedGroup.Children.Add(completedRows); body.Children.Add(completedGroup);
        UpdateRunRows(groups);
    }

    private void UpdateRunRows(NativeSessionRunGroups groups)
    {
        if (activeRows == null || completedRows == null) return;
        activeCount!.Text = groups.Active.Length.ToString();
        activeEmpty!.Visibility = groups.Active.Length == 0 ? Visibility.Visible : Visibility.Collapsed;
        completedGroup!.Visibility = groups.Completed.Length == 0 ? Visibility.Collapsed : Visibility.Visible;
        var label = design.Localization.Format("right.runs_completed", ("n", groups.Completed.Length.ToString()));
        completedCount!.Text = label.ToUpperInvariant();
        completedArrow!.Source = design.Icon(completedRunsOpen ? "chevron-down" : "chevron-right", 14).Source;
        AutomationProperties.SetName(completedToggle!, label);
        AutomationProperties.SetHelpText(completedToggle!, completedRunsOpen ? design.L("已展开", "Expanded") : design.L("已折叠", "Collapsed"));
        ReconcileRunRows(activeRows, groups.Active); ReconcileRunRows(completedRows, completedRunsOpen ? groups.Completed : []);
        var visible = groups.Active.Concat(completedRunsOpen ? groups.Completed : []).Select(r => r.Id).ToHashSet();
        foreach (var id in runCards.Keys.Where(id => !visible.Contains(id)).ToArray()) runCards.Remove(id);
    }

    private void ReconcileRunRows(StackPanel panel, NativeRun[] records)
    {
        for (var i = 0; i < records.Length; i++)
        {
            var record = records[i];
            if (!runCards.TryGetValue(record.Id, out var card))
                runCards[record.Id] = card = new(record, design, id => PresentRunAsync(id, false),
                    id => ChangeRunAsync(id, false), id => ChangeRunAsync(id, true));
            card.Update(record, model.Activity?.ReadOnly != false, model.ActivityBusy || runReview?.Mutating == true && runReview.RunId == record.Id);
            if (i < panel.Children.Count && ReferenceEquals(panel.Children[i], card)) continue;
            if (card.Parent is Panel old) old.Children.Remove(card);
            panel.Children.Insert(i, card);
        }
        while (panel.Children.Count > records.Length) panel.Children.RemoveAt(panel.Children.Count - 1);
    }

    private async Task ChangeRunAsync(string id, bool harvest)
    {
        var change = harvest ? model.HarvestRunAsync(id, lifetime.Token) : model.CancelRunAsync(id, lifetime.Token);
        Render(); await change; Render();
    }

    private TextBlock RunGroupLabel(string value)
    {
        var label = design.Text(value.ToUpperInvariant(), 12); label.Foreground = design.Brush("text-muted");
        label.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; return label;
    }

}
