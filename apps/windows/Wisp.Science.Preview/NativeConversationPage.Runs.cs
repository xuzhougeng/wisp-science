using System.Text.Json;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class NativeConversationPage
{
    private bool openingRunReview;
    private readonly Dictionary<string, (NativeRun Run, TextBlock Label)> runClocks = [];
    private readonly Func<string, Task>? reviewRun;
    private readonly Func<bool>? canShowRunReview;
    private void UpdateRunClocks()
    {
        foreach (var (_, (run, label)) in runClocks)
        {
            var elapsed = Math.Max(0, (run.EndedAt ?? DateTimeOffset.UtcNow.ToUnixTimeSeconds()) - (run.StartedAt ?? run.CreatedAt));
            label.Text = $"{run.ContextId} · {run.Kind} · {NativeToolPresentation.Duration((ulong)elapsed * 1000)}"
                + (run.ExitCode is { } exit ? $" · 退出码 {exit}" : "")
                + (run.LastPolledAt is { } poll ? $"\n最近轮询 {DateTimeOffset.FromUnixTimeSeconds(poll).ToLocalTime():HH:mm:ss}" : "");
        }
    }
    private FrameworkElement InlineRunCard(NativeRun run, bool embedded)
    {
        var body = new StackPanel { Spacing = 6, Padding = new Thickness(10), Background = design.Brush("bg-sunken"), CornerRadius = new CornerRadius(8) };
        body.Children.Add(design.Text(string.IsNullOrWhiteSpace(run.Title) ? run.Id : run.Title, 14));
        body.Children.Add(design.Text(NativeToolPresentation.RunState(run.Status), 12));
        var clock = new TextBlock { FontSize = design.FontSize(11), Foreground = design.Brush("text-muted"), TextWrapping = TextWrapping.Wrap };
        runClocks[run.Id] = (run, clock); body.Children.Add(clock);
        if (NativeRunPresentation.Progress(run) is { } progress)
        {
            body.Children.Add(new TextBlock { Text = progress.Label, TextWrapping = TextWrapping.Wrap, FontSize = design.FontSize(12) });
            body.Children.Add(new ProgressBar { Minimum = 0, Maximum = 100, Value = progress.Percent ?? 0,
                IsIndeterminate = progress.Percent == null && !WorkspaceConversationModel.RunTerminal(run.Status), Height = 3 });
        }
        if (!string.IsNullOrWhiteSpace(run.Command)) body.Children.Add(ToolText(run.Command));
        if (!string.IsNullOrWhiteSpace(run.RemoteWorkdir)) body.Children.Add(new TextBlock { Text = "工作目录 · " + run.RemoteWorkdir, TextWrapping = TextWrapping.Wrap, IsTextSelectionEnabled = true });
        if (NativeRunPresentation.Output(run) is { Length: > 0 } output) body.Children.Add(ToolText(output));
        if (!string.IsNullOrWhiteSpace(run.EnvSnapshotJson) && run.EnvSnapshotJson != "{}") body.Children.Add(new Expander { Header = "执行环境快照", Content = ToolText(run.EnvSnapshotJson), HorizontalAlignment = HorizontalAlignment.Stretch });
        if (!string.IsNullOrWhiteSpace(run.LastPollError)) body.Children.Add(new TextBlock { Text = "最近轮询失败 · " + run.LastPollError, TextWrapping = TextWrapping.Wrap, Foreground = design.Brush("clay-strong") });
        if (model.RunReadError(run.Id) is { } readError) body.Children.Add(new TextBlock { Text = readError, TextWrapping = TextWrapping.Wrap, Foreground = design.Brush("clay-strong") });
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4 };
        if (openRun != null) { var details = MessageAction("运行详情"); details.Click += (_, _) => openRun(run.Id); actions.Children.Add(details); }
        if (run.Status is "submitted" or "running" or "cancelling")
        {
            var cancel = MessageAction(run.Status == "cancelling" ? "强制取消" : "取消运行"); cancel.IsEnabled = model.CanCancelRun(run);
            cancel.Click += async (_, _) => await model.CancelInlineRunAsync(run, lifetime.Token); actions.Children.Add(cancel);
        }
        if (WorkspaceConversationModel.RunTerminal(run.Status))
        {
            if (run.Kind == "ssh_direct" && run.CleanedAt == null && reviewRun != null)
            { var review = MessageAction("结果与清理"); review.Click += async (_, _) => await reviewRun(run.Id); actions.Children.Add(review); }
            if (!embedded) { var dismiss = MessageAction("收起卡片"); dismiss.Click += (_, _) => model.DismissRunCard(run.Id); actions.Children.Add(dismiss); }
        }
        body.Children.Add(actions);
        if (model.RunError(run.Id) is { } error)
        {
            body.Children.Add(new TextBlock { Text = error, TextWrapping = TextWrapping.Wrap, Foreground = design.Brush("clay-strong") });
            var acknowledge = MessageAction("已核对运行状态"); acknowledge.Click += (_, _) => model.AcknowledgeRunAction(run.Id); body.Children.Add(acknowledge);
        }
        return body;
    }
    private async Task MaybeShowRunReviewPrompt()
    {
        if (disposed || openingRunReview || reviewRun == null || canShowRunReview?.Invoke() == false
            || historyDialog != null || queueDialog != null || queueMenu != null || historyMenu != null
            || composerOptionsOpen || acpSettingsOpen || acpCreationOpen || contextUsageOpen || selectionFlyout != null || XamlRoot == null
            || model.RunReviewPrompt is not { } prompt) return;
        openingRunReview = true;
        try
        {
            // Open the results browser directly. Persist dismissal only when
            // that browser closes, including manual entry points.
            if (model.TakeRunReviewPrompt(prompt)) await reviewRun(prompt.RunId);
        }
        finally { openingRunReview = false; }
    }
}
