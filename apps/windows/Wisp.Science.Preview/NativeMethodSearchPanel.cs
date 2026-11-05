using System.Globalization;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

/// <summary>Inline review of one frozen method-search contract in the environment's run list.</summary>
internal sealed class NativeMethodSearchPanel : StackPanel
{
    private readonly WispDesign design;
    private readonly NativeMethodReview review;
    private readonly CancellationToken token;
    private readonly Func<Task> refreshRuns;
    private readonly Func<NativeMethodDetail, NativeMethodOutput, Task> openOutput;
    private readonly Button inspect = new() { HorizontalAlignment = HorizontalAlignment.Stretch };
    private readonly StackPanel body = new() { Spacing = 9 };
    private readonly Border frame;
    private static readonly CultureInfo Numbers = CultureInfo.InvariantCulture;

    public NativeMethodSearchPanel(WispDesign design, NativeMethodReview review, CancellationToken token, Func<Task> refreshRuns,
        Func<NativeMethodDetail, NativeMethodOutput, Task> openOutput)
    {
        this.design = design; this.review = review; this.token = token; this.refreshRuns = refreshRuns; this.openOutput = openOutput;
        Spacing = 8; Margin = new(0, 4, 0, 0);
        design.ActionButton(inspect); inspect.MinHeight = 32;
        AutomationProperties.SetAutomationId(inspect, "method-search-inspect");
        inspect.Click += async (_, _) => {
            review.Expanded = !review.Expanded; Render();
            if (review.Expanded && (review.Stale || review.Detail == null)) await review.ExecuteAsync("inspect", token);
        };
        Children.Add(inspect);
        frame = Box(body, "bg-sunken", 10, 8); Children.Add(frame); Render();
        Loaded += async (_, _) => {
            review.Changed += Render;
            if (review.Expanded && (review.Detail == null || review.Stale) && !review.Unconfirmed && review.Error == null)
                await review.ExecuteAsync("inspect", token);
            Render();
        };
        Unloaded += (_, _) => review.Changed -= Render;
    }
    private string T(string key) => design.T("method_search." + key);
    private TextBlock Label(string? value, double size, string color = "text-muted", bool mono = false, bool strong = false)
    {
        var text = design.Text(value ?? "", size); text.Foreground = design.Brush(color); text.TextWrapping = TextWrapping.Wrap;
        if (mono) text.FontFamily = design.Font(true);
        if (strong) text.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        return text;
    }
    private Border Box(UIElement content, string fill, double padding, double radius) => new() {
        Child = content, Padding = new(padding), CornerRadius = new(radius), Background = design.Brush(fill),
        BorderBrush = design.Brush("border"), BorderThickness = new(1) };
    private static string Prefix(string? text, int length = 12) => string.IsNullOrEmpty(text) ? "—" : text[..Math.Min(length, text.Length)];
    private static string Score(double? value) => value?.ToString("F6", Numbers) ?? "—";
    private static NativeActionWrap Wrap() => new() { Gap = 7, RowGap = 5, VerticallyCentered = true };
    private Button SmallButton(string title, bool primary = false)
    {
        var button = new Button { Content = title }; design.ActionButton(button, primary); button.Padding = new(9, 4, 9, 4); button.MinHeight = 27;
        if (button.Content is TextBlock caption) design.BindTypography(caption, 10.5);
        return button;
    }
    private void Render()
    {
        if (token.IsCancellationRequested) return;
        var toggleName = T(review.Expanded ? "hide" : "inspect");
        if (inspect.Content is not TextBlock caption || caption.Text != toggleName)
            inspect.Content = Label(toggleName, 12, "text", strong: true);
        AutomationProperties.SetName(inspect, toggleName);
        frame.Visibility = review.Expanded ? Visibility.Visible : Visibility.Collapsed;
        body.Children.Clear(); if (!review.Expanded) return;
        var head = new Grid(); head.ColumnDefinitions.Add(new()); head.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var title = Label(T("contract"), 12, "text", strong: true); title.VerticalAlignment = VerticalAlignment.Center; head.Children.Add(title);
        var refresh = design.ToolButton(design.T("runs.refresh"), "sync"); refresh.Width = refresh.Height = 24; refresh.Padding = new(3); refresh.MinWidth = refresh.MinHeight = 0;
        AutomationProperties.SetAutomationId(refresh, "method-search-refresh"); refresh.IsEnabled = !review.Busy;
        refresh.Click += async (_, _) => await review.ExecuteAsync("inspect", token); Grid.SetColumn(refresh, 1); head.Children.Add(refresh); body.Children.Add(head);
        if (review.Busy) body.Children.Add(Label(T("loading"), 11.5, "text-faint"));
        if (review.Error != null || review.Unconfirmed)
            body.Children.Add(Label((review.Unconfirmed ? design.L("操作未确认，请刷新核对。", "Action not confirmed. Refresh to check.") + " " : "") + review.Error, 11, "err"));
        if (review.Detail is not { } d) return;
        var spec = d.Spec; var audit = d.Audit; var b = audit.Baseline;
        var approval = new StackPanel { Spacing = 8 }; var ids = Wrap();
        ids.Children.Add(NativeWorkspacePanel.RunStatusBadge(design, d.Run.Status, design.T("runs.status." + d.Run.Status)));
        ids.Children.Add(CodeLabel("spec " + Prefix(d.State.SpecArtifactVersionId), d.State.SpecArtifactVersionId));
        ids.Children.Add(CodeLabel("audit " + Prefix(d.AuditVersion), d.AuditVersion)); approval.Children.Add(ids);
        approval.Children.Add(Label(spec.Objective, 11.5));
        approval.Children.Add(Fields(2, [
            ("target", spec.Target.SourcePath + "::" + spec.Target.Symbol), ("evaluator", spec.Evaluator.EntryPath),
            ("metric", spec.Metrics.Primary + " · " + spec.Metrics.Direction), ("context", d.Run.ContextId),
            ("baseline", Score(b.MedianPrimary) + " ± " + Score(b.Spread)), ("noise", Score(b.NoiseFloor)),
            ("reachability", audit.SentinelReachable ? design.L("是", "Yes") : design.L("否", "No")),
            ("budget", $"{spec.Budget.MaxCandidates} · {spec.Budget.MaxCostMicrounits} · {spec.Budget.MaxWallSeconds}s · eval {spec.Budget.MaxEvaluatorSeconds}s"),
            ("protected", spec.ProtectedPaths.Length.ToString(Numbers)) ], audit.SentinelReachable));
        if (spec.Metrics.Guardrails.Length > 0)
        {
            var tags = Wrap(); tags.Children.Add(Label(T("guardrails"), 10, "text-faint"));
            foreach (var g in spec.Metrics.Guardrails) { var pill = Box(Label($"{g.Metric} {g.Op} {g.Value.ToString(Numbers)}", 9.5, mono: true), "bg-sunken", 2, 12); pill.Padding = new(6, 2, 6, 2); tags.Children.Add(pill); }
            approval.Children.Add(tags);
        }
        foreach (var finding in spec.Constraints) approval.Children.Add(Label("• " + finding, 10.5));
        var actions = Wrap();
        foreach (var op in NativeMethodReview.Actions(d.Run.Status))
        {
            var button = SmallButton(T(op), op is "start" or "resume");
            if (op == "cancel" && button.Content is TextBlock cancel) cancel.Foreground = design.Brush("err");
            button.IsEnabled = review.Writable && !review.Busy && !review.Stale && !review.Unconfirmed;
            AutomationProperties.SetAutomationId(button, "method-search-" + op);
            button.Click += async (_, _) => {
                await review.ExecuteAsync(op, token);
                if (!token.IsCancellationRequested) await refreshRuns();
            };
            actions.Children.Add(button);
        }
        if (actions.Children.Count > 0) approval.Children.Add(actions);
        body.Children.Add(Box(approval, "bg-elev", 9, 8));
        var p = NativeMethodProgressDetail.Parse(d.Run.ProgressJson);
        body.Children.Add(Fields(3, [("phase", p.Phase ?? ""), ("baseline", Score(p.BaselinePrimary)), ("best", Score(p.BestPrimary)),
            ("candidates", p.CandidateCount.ToString(Numbers)), ("success_failed", $"{p.SuccessfulCount} / {p.FailedCount}"), ("cost", p.CostMicrounits.ToString(Numbers)),
            ("strategy", p.CurrentStrategy ?? "—"), ("checkpoint", p.LastCheckpointAt is long ts ? $"{Math.Max(0, DateTimeOffset.UtcNow.ToUnixTimeSeconds() - ts)}s" : "—"),
            ("best_candidate", Prefix(p.BestCandidateId)) ]));
        if (d.Outputs.Length > 0)
        {
            body.Children.Add(Label(T("outputs"), 10.5, "text", strong: true)); var links = Wrap();
            foreach (var o in d.Outputs) {
                var name = string.IsNullOrWhiteSpace(o.LogicalOutputKey) ? o.Role : o.LogicalOutputKey;
                var button = SmallButton(name); button.CornerRadius = new(12); button.Content = Label(name, 11, mono: true); button.MinHeight = 0; button.Padding = new(8, 2, 8, 2);
                AutomationProperties.SetName(button, name); ToolTipService.SetToolTip(button, o.SourcePath);
                button.Click += async (_, _) => { if (!token.IsCancellationRequested && ReferenceEquals(review.Detail, d)) await openOutput(d, o); }; links.Children.Add(button);
            }
            body.Children.Add(links);
        }
        if (d.Candidates.Length > 0) Disclosure(design.Localization.Format("method_search.lineage", ("count", d.Candidates.Length.ToString(Numbers))),
            true, () => d.Candidates.Select(c => {
                var col = new StackPanel { Spacing = 3 }; var row = Wrap();
                foreach (var value in new[] { "#" + c.Sequence, c.Family, c.StrategyKey, c.PrimaryScore == null ? c.Status : Score(c.PrimaryScore),
                    c.ChangedLines == null ? null : "Δ " + c.ChangedLines, c.RuntimeMs == null ? null : c.RuntimeMs + " ms",
                    c.ParentCandidateId == null ? null : "← " + Prefix(c.ParentCandidateId, 8) }.OfType<string>()) row.Children.Add(CodeLabel(value));
                col.Children.Add(row); if (!string.IsNullOrEmpty(c.Rationale)) col.Children.Add(Label(c.Rationale, 10.5));
                if (!string.IsNullOrEmpty(c.Error)) col.Children.Add(Label(c.Error, 10.5, "err")); return Box(col, "bg-elev", 6, 5);
            }));
        if (d.Strategies.Length > 0) Disclosure(T("strategies"), false, () => d.Strategies.Select(s => {
            var row = Wrap(); foreach (var value in new[] {s.StrategyKey, s.Category, s.Weight.ToString("F3", Numbers), $"{s.Improvements} / {s.Attempts}"}) row.Children.Add(CodeLabel(value));
            return Box(row, "bg-elev", 6, 5);
        }));
        var integrity = Wrap(); integrity.Children.Add(CodeLabel(d.State.ControlState, d.State.SpecSha256));
        foreach (var value in new[] { d.State.ResultStatus ?? T("pending"), $"{b.SuccessfulRepetitions} / {b.Repetitions}", (b.FailureRate * 100).ToString("F1", Numbers) + "%",
            audit.ProtectedFiles.Length + " " + T("protected"), audit.Findings.Length + " " + T("findings") }) integrity.Children.Add(Label(value, 9, "text-faint"));
        integrity.Children.Add(CodeLabel(Prefix(spec.Target.SourceArtifactVersionId), audit.TargetSourceSha256)); integrity.Children.Add(CodeLabel($"{spec.Evaluator.Repetitions}×{spec.Evaluator.TimeoutSeconds}s", spec.Evaluator.ArtifactVersionId));
        integrity.Children.Add(Label(T(spec.FinalVerification != null ? "final_enabled" : "validation_only"), 9, "text-faint")); body.Children.Add(integrity);
    }
    private TextBlock CodeLabel(string value, string? tooltip = null)
    { var tag = Label(value, 9.5, "text-faint", true); if (tooltip != null) ToolTipService.SetToolTip(tag, tooltip); return tag; }
    private Grid Fields(int columns, (string Key, string Value)[] fields, bool? reachable = null)
    {
        var grid = new Grid { ColumnSpacing = 6, RowSpacing = 6 };
        foreach (var field in fields) {
            var slot = new StackPanel { Spacing = 2 }; slot.Children.Add(Label(T(field.Key), 9.5, "text-faint"));
            if (field.Key == "reachability" && reachable.HasValue)
            {
                var icon = design.Icon(reachable.Value ? "check" : "close", 12);
                icon.HorizontalAlignment = HorizontalAlignment.Left; AutomationProperties.SetName(icon, field.Value);
                slot.Children.Add(icon);
            }
            else slot.Children.Add(Label(field.Value, 10.5, "text", columns == 3 || field.Key is "target" or "evaluator" or "context"));
            grid.Children.Add(Box(slot, columns == 3 ? "bg-elev" : "bg-sunken", 6, 5));
        }
        void ArrangeFields()
        {
            int n = (XamlRoot?.Size.Width ?? 1000) <= 760 ? 1 : columns;
            if (grid.ColumnDefinitions.Count == n) return;
            grid.ColumnDefinitions.Clear(); grid.RowDefinitions.Clear();
            for (int i = 0; i < n; i++) grid.ColumnDefinitions.Add(new());
            for (int i = 0; i < fields.Length; i++) {
                if (i % n == 0) grid.RowDefinitions.Add(new() { Height = GridLength.Auto });
                Grid.SetColumn((FrameworkElement)grid.Children[i], i % n); Grid.SetRow((FrameworkElement)grid.Children[i], i / n);
            }
        }
        grid.SizeChanged += (_, _) => ArrangeFields(); ArrangeFields(); return grid;
    }
    private void Disclosure(string title, bool candidates, Func<IEnumerable<Border>> entries)
    {
        var heading = new Button { HorizontalAlignment = HorizontalAlignment.Left, MinHeight = 0, Padding = new(0, 2, 4, 2) };
        design.QuietButton(heading); heading.MinHeight = 0; heading.Padding = new(0, 2, 4, 2);
        var list = new StackPanel { Spacing = 5 }; var scroll = new ScrollViewer { MaxHeight = 220, Content = list };
        void Show(bool expanded) {
            var line = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 5 };
            line.Children.Add(design.Icon(expanded ? "chevron-down" : "chevron-right", 12)); line.Children.Add(Label(title, 10.5, strong: true));
            heading.Content = line; AutomationProperties.SetName(heading, title);
            scroll.Visibility = expanded ? Visibility.Visible : Visibility.Collapsed;
            if (expanded && list.Children.Count == 0) foreach (var item in entries()) list.Children.Add(item);
        }
        heading.Click += (_, _) => {
            if (candidates) Show(review.ShowCandidates = !review.ShowCandidates); else Show(review.ShowStrategies = !review.ShowStrategies);
        };
        Show(candidates ? review.ShowCandidates : review.ShowStrategies); body.Children.Add(heading); body.Children.Add(scroll);
    }
}
