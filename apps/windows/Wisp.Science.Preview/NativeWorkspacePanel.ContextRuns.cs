using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class NativeWorkspacePanel
{
    private readonly Action<NativeRun>? useRunInPublication;
    // This is the environment list (possibly across several conversations), not the Runs tab.
    private void MountContextRuns(StackPanel section, string context, Popup popup)
    {
        var scope = CancellationTokenSource.CreateLinkedTokenSource(lifetime.Token);
        var token = scope.Token;
        var heading = ContextHeading(design.T("contexts.runs"));
        heading.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var count = design.Text("", 12); count.Foreground = design.Brush("text-faint");
        count.VerticalAlignment = VerticalAlignment.Center; Grid.SetColumn(count, 1); heading.Children.Add(count);
        var refresh = design.ToolButton(design.T("runs.refresh"), "sync"); refresh.Content = design.Icon("sync", 14);
        refresh.Width = refresh.Height = 24; refresh.MinWidth = refresh.MinHeight = 0; refresh.Padding = new(3);
        Grid.SetColumn(refresh, 2); heading.Children.Add(refresh); section.Children.Add(heading);
        var rows = new StackPanel { Spacing = 10, Visibility = Visibility.Collapsed };
        var notice = Mute(design.T("loading")); section.Children.Add(notice); section.Children.Add(rows);
        var empty = ContextEmpty("runs.empty", elevated: true); empty.Visibility = Visibility.Collapsed; section.Children.Add(empty);
        var cards = new Dictionary<string, (NativeRun Record, bool Pending, string? Error, NativeRunArtifact[] Links, FrameworkElement Card)>();
        var links = Array.Empty<NativeRunArtifact>().ToLookup(a => a.RunId);
        (string Id, long? HarvestedAt)[]? linkRevision = null;
        var linkNotice = Mute(""); linkNotice.Foreground = design.Brush("warn"); linkNotice.Visibility = Visibility.Collapsed;
        section.Children.Add(linkNotice);
        var openIds = new HashSet<string>(); var busy = false; var readVersion = 0;
        var feedback = Mute(""); feedback.Visibility = Visibility.Collapsed; section.Children.Insert(section.Children.IndexOf(rows), feedback);
        var timer = DispatcherQueue.CreateTimer(); timer.Interval = TimeSpan.FromSeconds(5);
        async Task Read(bool manual = false, bool supersede = false)
        {
            if ((busy && !supersede) || token.IsCancellationRequested || !popup.IsOpen) return;
            var version = ++readVersion;
            busy = true; refresh.IsEnabled = false;
            try
            {
                var result = await model.ReadContextRunsAsync(context, token, manual);
                if (token.IsCancellationRequested || !popup.IsOpen || version != readVersion) return;
                var revision = result.Select(r => (r.Id, r.HarvestedAt)).ToArray();
                if (manual || linkRevision == null || !revision.SequenceEqual(linkRevision))
                {
                    try
                    {
                        var records = result.Length == 0 ? [] : await model.ReadContextRunArtifactsAsync(context, token);
                        if (token.IsCancellationRequested || !popup.IsOpen || version != readVersion) return;
                        links = records.ToLookup(a => a.RunId); linkNotice.Visibility = Visibility.Collapsed;
                    }
                    catch (Exception ex)
                    {
                        if (token.IsCancellationRequested || !popup.IsOpen || version != readVersion) return;
                        links = Array.Empty<NativeRunArtifact>().ToLookup(a => a.RunId);
                        linkNotice.Text = design.L("无法读取产物，请刷新重试。", "Could not load artifacts. Refresh to retry.") + " " + ex.Message;
                        linkNotice.Visibility = Visibility.Visible;
                    }
                    linkRevision = revision;
                }
                for (int i = 0; i < result.Length; i++)
                {
                    var run = result[i];
                    var pending = model.RunActionPending(run.Id); var error = model.RunActionError(run.Id);
                    var produced = links[run.Id];
                    if (!cards.TryGetValue(run.Id, out var entry) || entry.Record with { LastPolledAt = run.LastPolledAt } != run
                        || entry.Pending != pending || entry.Error != error || !entry.Links.SequenceEqual(produced))
                    {
                        var outputs = produced.ToArray();
                        if (entry.Card != null) rows.Children.Remove(entry.Card);
                        cards[run.Id] = entry = (run, pending, error, outputs, ContextRunCard(run, context, openIds, outputs, token, popup, async action =>
                        {
                            if (cards.TryGetValue(run.Id, out var existing))
                                cards[run.Id] = (existing.Record, true, existing.Error, existing.Links, existing.Card);
                            var ok = await model.ChangeContextRunAsync(context, run, action, token);
                            if (token.IsCancellationRequested || !popup.IsOpen) return;
                            if (ok && action == ContextRunAction.Cleanup) { feedback.Text = design.T("runs.cleanup_done"); feedback.Visibility = Visibility.Visible; }
                            await Read(supersede: true);
                        }, () => Read(supersede: true)));
                    }
                    if (i < rows.Children.Count && rows.Children[i] == entry.Card) continue;
                    if (entry.Card.Parent == rows) rows.Children.Remove(entry.Card);
                    rows.Children.Insert(i, entry.Card);
                }
                while (rows.Children.Count > result.Length) rows.Children.RemoveAt(rows.Children.Count - 1);
                var ids = result.Select(r => r.Id).ToHashSet(); openIds.IntersectWith(ids);
                foreach (var id in cards.Keys.Where(id => !ids.Contains(id)).ToArray()) cards.Remove(id);
                count.Text = result.Length.ToString(); empty.Visibility = result.Length == 0 ? Visibility.Visible : Visibility.Collapsed;
                rows.Visibility = result.Length == 0 ? Visibility.Collapsed : Visibility.Visible;
                notice.Visibility = Visibility.Collapsed;
                timer.Interval = TimeSpan.FromSeconds(result.Any(r => r.Status is "running" or "submitted" or "cancelling" && !string.IsNullOrEmpty(r.ProgressJson) && r.ProgressJson != "{}") ? 1 : 5);
            }
            catch (OperationCanceledException) when (token.IsCancellationRequested) { }
            catch (Exception ex)
            {
                if (!token.IsCancellationRequested && popup.IsOpen && version == readVersion)
                {
                    empty.Visibility = Visibility.Collapsed; notice.Text = design.Localization.Format("runs.refresh_failed", ("error", ex.Message));
                    notice.Foreground = design.Brush("warn"); notice.Visibility = Visibility.Visible;
                }
            }
            finally { if (version == readVersion) { busy = false; if (!token.IsCancellationRequested) refresh.IsEnabled = true; } }
        }
        refresh.Click += async (_, _) => { feedback.Visibility = Visibility.Collapsed; await Read(manual: true); };
        timer.Tick += async (_, _) => await Read();
        popup.Opened += async (_, _) => { timer.Start(); await Read(); };
        popup.Closed += (_, _) => { timer.Stop(); scope.Cancel(); scope.Dispose(); };
    }

    private Border ContextRunCard(NativeRun run, string context, HashSet<string> openIds, NativeRunArtifact[] outputs, CancellationToken token, Popup popup,
        Func<ContextRunAction, Task> change, Func<Task> refresh)
    {
        var stack = new StackPanel { Spacing = 4 };
        var title = design.Text(string.IsNullOrWhiteSpace(run.Title) ? run.Id : run.Title, 13);
        title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; title.TextTrimming = TextTrimming.CharacterEllipsis; title.MaxLines = 1;
        // Allow long titles to shrink before they push the status out of the card.
        var wrap = new Grid { ColumnSpacing = 8 }; wrap.ColumnDefinitions.Add(new()); wrap.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); wrap.Children.Add(title);
        var badge = RunStatusBadge(design, run.Status, run.Status);
        Grid.SetColumn(badge, 1); wrap.Children.Add(badge);
        var head = new Grid { ColumnSpacing = 12 };
        head.ColumnDefinitions.Add(new()); head.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var meta = new StackPanel { Spacing = 4 }; meta.Children.Add(wrap);
        meta.Children.Add(Mute(run.ContextId + " · " + run.Kind + (run.ExitCode is { } exit ? " · exit " + exit : "")));
        head.Children.Add(meta);
        var actions = new NativeActionWrap { Gap = 6, RightAligned = true, VerticalAlignment = VerticalAlignment.Top };
        if (useRunInPublication != null)
        {
            var use = new Button { Content = design.T("publication.use"), VerticalAlignment = VerticalAlignment.Top };
            design.ActionButton(use); use.Padding = new(10, 2, 10, 2); use.MinHeight = 26;
            use.CornerRadius = new(5); use.Background = design.Brush("clay-soft");
            if (use.Content is TextBlock caption) { design.BindTypography(caption, 11); caption.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; caption.Foreground = design.Brush("clay-strong"); }
            use.Click += (_, _) => {
                if (disposed || token.IsCancellationRequested || contextDialog != popup || !popup.IsOpen) return;
                popup.IsOpen = false;
                useRunInPublication(run);
            };
            actions.Children.Add(use);
        }
        var writes = new List<Button>();
        Button IconAction(string key, string icon) {
            var button = design.ToolButton(design.T(key), icon); button.Width = button.Height = 26;
            button.Padding = new(4); button.MinWidth = button.MinHeight = 0; actions.Children.Add(button); return button;
        }
        void MutationButton(string key, string icon, ContextRunAction action) {
            var button = IconAction(key, icon); writes.Add(button);
            button.IsEnabled = run.CanManage == true && !model.RunActionPending(run.Id);
            button.Opacity = button.IsEnabled ? 1 : .45; button.IsEnabledChanged += (_, _) => button.Opacity = button.IsEnabled ? 1 : .45;
            button.Click += async (_, _) => {
                if (!popup.IsOpen || contextDialog != popup || token.IsCancellationRequested || model.RunActionPending(run.Id)) return;
                foreach (var b in writes) b.IsEnabled = false;
                await change(action);
            };
        }
        if (run.CanManage.HasValue && WorkspacePanelModel.CancelAction(run) is { } cancel)
            MutationButton(cancel == ContextRunAction.ForceCancel ? "runs.force_cancel" : "runs.cancel", "close", cancel);
        if (run.CanManage.HasValue && WorkspacePanelModel.HasWorkspaceActions(run))
        {
            var results = IconAction("run_review.open", "folder");
            results.Click += async (_, _) => {
                if (contextDialog == popup && popup.IsOpen && !token.IsCancellationRequested)
                    await ShowContextReviewAsync(run, context, popup, results, refresh, token);
            };
            MutationButton("runs.cleanup", "trash", ContextRunAction.Cleanup);
        }
        if (run.CleanedAt != null) actions.Children.Add(Mute(design.T("runs.cleaned")));
        Grid.SetColumn(actions, 1); head.Children.Add(actions);
        // Keep the title readable when the action group no longer fits beside it.
        head.RowDefinitions.Add(new() { Height = GridLength.Auto }); head.RowDefinitions.Add(new() { Height = GridLength.Auto });
        head.SizeChanged += (_, e) => {
            var narrow = e.NewSize.Width < 360;
            Grid.SetColumnSpan(meta, narrow ? 2 : 1); Grid.SetColumn(actions, narrow ? 0 : 1); Grid.SetColumnSpan(actions, narrow ? 2 : 1); Grid.SetRow(actions, narrow ? 1 : 0);
        };
        stack.Children.Add(head);
        if (model.RunActionError(run.Id) is { } problem) {
            var warning = Mute(design.L("操作未确认，请刷新核对。", "Action not confirmed. Refresh to check.") + " " + problem);
            warning.Foreground = design.Brush("warn"); stack.Children.Add(warning);
        }
        if (NativeRunPresentation.Progress(run, design.Localization) is { } progress) stack.Children.Add(ContextProgress(progress));
        if (run.Kind == "method_search")
        {
            var review = model.ContextMethodReview(context, run);
            stack.Children.Add(new NativeMethodSearchPanel(design, review, token, refresh, async (detail, output) =>
            {
                if (disposed || token.IsCancellationRequested || contextDialog != popup || !popup.IsOpen) return;
                var reader = model.MethodOutputPreview(context, detail);
                popup.IsOpen = false;
                await OpenArtifactDialogAsync(output.ArtifactVersionId, reader);
            }));
        }
        if (!string.IsNullOrWhiteSpace(run.RemoteWorkdir)) stack.Children.Add(Mute(design.T("runs.remote_workdir") + "   " + run.RemoteWorkdir));
        foreach (var error in new[] { run.CleanupError, run.LastPollError }.Where(s => !string.IsNullOrWhiteSpace(s)))
        { var warning = design.Text(error!, 11.5); warning.Foreground = design.Brush("err"); stack.Children.Add(warning); }

        var latest = new StackPanel { Spacing = 5, Visibility = Visibility.Collapsed };
        var toggle = new Button { HorizontalAlignment = HorizontalAlignment.Left }; design.QuietButton(toggle);
        toggle.Padding = new(0, 2, 4, 2); toggle.MinHeight = 20;
        var toggleRow = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4 };
        var arrow = design.Icon("chevron-right", 12); toggleRow.Children.Add(arrow); toggleRow.Children.Add(Mute(design.T("runs.output")));
        toggle.Content = toggleRow; AutomationProperties.SetName(toggle, design.T("runs.output")); stack.Children.Add(toggle); stack.Children.Add(latest);
        if (outputs.Length != 0)
        {
            var strip = new NativeActionWrap { Gap = 6, VerticallyCentered = true, Margin = new(0, 4, 0, 0) };
            var caption = design.Text(design.T("runs.artifacts"), 11); caption.Foreground = design.Brush("text-faint"); strip.Children.Add(caption);
            foreach (var link in outputs)
            {
                var label = string.IsNullOrWhiteSpace(link.Label) ? link.ArtifactId : link.Label;
                var chip = new Button(); design.ActionButton(chip); chip.Padding = new(8, 2, 8, 2); chip.MinWidth = chip.MinHeight = 0; chip.CornerRadius = new(16);
                var text = design.Text(label, 11.5); text.FontFamily = design.Font(true); text.MaxLines = 1; text.TextTrimming = TextTrimming.CharacterEllipsis;
                chip.Content = text; ToolTipService.SetToolTip(chip, label); AutomationProperties.SetName(chip, label);
                chip.Click += async (_, _) =>
                {
                    if (disposed || token.IsCancellationRequested || contextDialog != popup || !popup.IsOpen) return;
                    var reader = model.ContextArtifactPreview(link);
                    popup.IsOpen = false;
                    await OpenArtifactDialogAsync(link.ArtifactId, reader);
                };
                strip.Children.Add(chip);
            }
            stack.Children.Add(strip);
        }
        var loading = false; var loaded = false;
        async Task ShowOutput(bool open)
        {
            latest.Visibility = open ? Visibility.Visible : Visibility.Collapsed;
            arrow.Source = design.Icon(open ? "chevron-down" : "chevron-right", 12).Source;
            if (!open || loading || loaded || token.IsCancellationRequested) return;
            loading = true; latest.Children.Clear(); latest.Children.Add(Mute(design.T("loading")));
            try
            {
                var detail = await model.ReadContextRunDetailAsync(context, run.Id, token);
                if (token.IsCancellationRequested || !stack.IsLoaded) return;
                loaded = true; latest.Children.Clear();
                if (!string.IsNullOrWhiteSpace(detail.Command)) { var command = Mute(detail.Command); command.FontFamily = design.Font(true); command.IsTextSelectionEnabled = true; latest.Children.Add(command); }
                if (NativeRunPresentation.Output(detail) is { Length: > 0 } output)
                {
                    var text = design.Text(output, 11); text.FontFamily = design.Font(true); text.IsTextSelectionEnabled = true;
                    latest.Children.Add(new Border { CornerRadius = new(5), Background = design.Brush("bg-sunken"), Padding = new(10, 9, 10, 9),
                        BorderBrush = design.Brush("border"), BorderThickness = new(1), Child = new ScrollViewer { MaxHeight = 220, Content = text } });
                }
            }
            catch (Exception) when (token.IsCancellationRequested) { }
            catch (Exception ex)
            {
                if (!stack.IsLoaded) return;
                latest.Children.Clear(); latest.Children.Add(Mute(design.Localization.Format("runs.detail_failed", ("error", ex.Message))));
                var retry = new Button { Content = design.L("重试", "Retry"), HorizontalAlignment = HorizontalAlignment.Left };
                design.QuietButton(retry); retry.Click += async (_, _) => await ShowOutput(true); latest.Children.Add(retry);
            }
            finally { loading = false; }
        }
        toggle.Click += async (_, _) => { if (!openIds.Remove(run.Id)) openIds.Add(run.Id); await ShowOutput(openIds.Contains(run.Id)); };
        if (openIds.Contains(run.Id)) stack.Loaded += async (_, _) => await ShowOutput(true);
        return new Border { Child = stack, CornerRadius = new(8), Padding = new(14, 12, 14, 12),
            Background = design.Brush("bg-elev"), BorderBrush = design.Brush("border-strong"), BorderThickness = new(1) };
    }

    internal static Grid RunStatusBadge(WispDesign design, string status, string caption)
    {
        var (ink, fill, opacity) = status switch {
            "draft" => ("warn", "warn", .13), "paused" => ("text-muted", "text-muted", .12),
            "succeeded" => ("ok", "ok", .12), "failed" or "timed_out" => ("err", "err", .12),
            "running" or "submitted" or "cancelling" => ("clay-strong", "clay-soft", 1d),
            _ => ("text-faint", "bg-sunken", 1d)
        };
        var grid = new Grid { VerticalAlignment = VerticalAlignment.Center };
        grid.Children.Add(new Border { Background = design.Brush(fill), Opacity = opacity, CornerRadius = new(12) });
        var text = design.Text(caption.ToUpperInvariant(), 10); text.Margin = new(7, 2, 7, 2);
        text.Foreground = design.Brush(ink); text.FontWeight = Microsoft.UI.Text.FontWeights.Bold;
        grid.Children.Add(text); return grid;
    }

    private FrameworkElement ContextProgress(NativeRunProgress value)
    {
        TextBlock Label(string text, double size, string color, bool mono = false)
        {
            var label = design.Text(text, size); label.Foreground = design.Brush(color); if (mono) label.FontFamily = design.Font(true);
            return label;
        }
        if (value is NativeMethodProgress method)
        {
            var grid = new Grid { ColumnSpacing = 8, Padding = new(8, 6, 8, 6) };
            grid.ColumnDefinitions.Add(new()); grid.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); grid.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            grid.Children.Add(Label(method.Phase, 10.5, "text-muted", true));
            var counts = Label(method.Counts, 10.5, "text-muted", true); Grid.SetColumn(counts, 1); grid.Children.Add(counts);
            var best = Label(method.Best, 10.5, "text", true); best.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; Grid.SetColumn(best, 2); grid.Children.Add(best);
            var shade = new Grid(); shade.Children.Add(new Border { Background = design.Brush("ok"), Opacity = .06 }); shade.Children.Add(grid);
            return new Border { CornerRadius = new(5), Background = design.Brush("bg-sunken"), Child = shade, Margin = new(0, 3, 0, 0) };
        }
        var transfer = (NativeTransferProgress)value;
        var group = new StackPanel { Spacing = 5, Margin = new(0, 3, 0, 0) };
        var header = new Grid { ColumnSpacing = 8 }; header.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); header.ColumnDefinitions.Add(new());
        var phase = Label(transfer.Phase, 11.5, "text"); phase.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; header.Children.Add(phase);
        if (!string.IsNullOrEmpty(transfer.File))
        {
            var file = Label(transfer.File, 11.5, "text-muted"); file.MaxLines = 1; file.TextTrimming = TextTrimming.CharacterEllipsis;
            Grid.SetColumn(file, 1); header.Children.Add(file); ToolTipService.SetToolTip(file, transfer.File);
        }
        group.Children.Add(header);
        var meta = new NativeActionWrap { Gap = 8, RowGap = 2 };
        for (var i = 0; i < transfer.Details.Length; i++)
            meta.Children.Add(Label((i == 0 ? "" : "· ") + transfer.Details[i], 10.5, "text-faint", true));
        group.Children.Add(meta);
        var bar = new ProgressBar { Minimum = 0, Maximum = 100, Value = transfer.Percent ?? 0, IsIndeterminate = transfer.Percent == null,
            Height = 7, MinHeight = 7, Foreground = design.Brush("clay-strong"), Background = design.Brush("border"), CornerRadius = new(4) };
        bar.Resources["ProgressBarTrackHeight"] = 7d;
        bar.Resources["ProgressBarTrackCornerRadius"] = new CornerRadius(4);
        AutomationProperties.SetName(bar, transfer.Phase); group.Children.Add(bar);
        return group;
    }
}
