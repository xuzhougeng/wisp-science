using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed class NativeSessionDeletePage : WorkspaceSheet
{
    private readonly WorkspaceSessionDeletion model;
    private readonly NativeSessionCheck children, artifacts;
    private readonly StackPanel childrenDetails = new() { Spacing = 8 }, artifactDetails = new() { Spacing = 8 };
    private readonly TextBlock error, loading;
    private readonly Button confirm, cancel, retry;
    private bool disposed;
    public NativeSessionDeletePage(WorkspaceSessionDeletion model, WispDesign design, Action close)
        : base(design, design.T("confirm.title"), close, modal: true, modalWidth: 480)
    {
        this.model = model;
        Body.Children.Add(design.Text(design.T(model.SessionIds.Length == 1 ? "session.delete_confirm" : "session.delete_many_confirm").Replace("{n}", model.SessionIds.Length.ToString()), 14));
        children = new NativeSessionCheck(design, design.T("session.children.delete"), 14, wrap: true) { Padding = new Thickness(0), Margin = new Thickness(0, 2, 0, 0) };
        children.Click += async (_, _) => await model.ChooseChildrenAsync(!model.IncludeChildren); Body.Children.Add(children); Body.Children.Add(childrenDetails);
        artifacts = new NativeSessionCheck(design, design.T("session.artifacts.delete"), 14, wrap: true) { Padding = new Thickness(0), Margin = new Thickness(0, 2, 0, 0) };
        artifacts.Click += async (_, _) => await model.ChooseArtifactsAsync(!model.IncludeArtifacts); Body.Children.Add(artifacts); Body.Children.Add(artifactDetails);
        loading = Mute(design.T("loading")); Body.Children.Add(loading);
        error = design.Text("", 13); error.Foreground = design.Brush("err"); Body.Children.Add(error);
        retry = new Button { Content = design.Text(design.T("session.artifacts.refresh"), 14), HorizontalAlignment = HorizontalAlignment.Left };
        retry.Click += async (_, _) => await model.LoadAsync(); Body.Children.Add(retry);
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, HorizontalAlignment = HorizontalAlignment.Right };
        cancel = new Button { Content = design.Text(design.T("settings.cancel"), 14), Padding = new Thickness(16, 9, 16, 9), CornerRadius = new CornerRadius(10) }; cancel.Click += (_, _) => HandleEscape(); actions.Children.Add(cancel);
        confirm = new Button { Content = design.Text(design.T("ctx.delete_session"), 14), Padding = new Thickness(16, 9, 16, 9), CornerRadius = new CornerRadius(10) };
        confirm.Background = design.Brush("clay"); confirm.Foreground = design.Brush("on-clay"); ((TextBlock)confirm.Content).Foreground = design.Brush("on-clay");
        confirm.Click += async (_, _) => { if (await model.DeleteAsync() && !disposed) close(); }; actions.Children.Add(confirm);
        Footer.Children.Add(actions); Footer.Visibility = Visibility.Visible;
        if (Content is Grid overlay) overlay.PointerPressed += (_, e) => { if (ReferenceEquals(e.OriginalSource, overlay)) HandleEscape(); };
        model.Changed += Refresh; Refresh();
    }
    private void Refresh()
    {
        if (disposed) return;
        children.SetSelected(model.IncludeChildren); artifacts.SetSelected(model.IncludeArtifacts);
        children.IsEnabled = artifacts.IsEnabled = !model.Deleting && !model.MutationUnconfirmed;
        children.Visibility = model.Preview is { Children.IsEmpty: false } ? Visibility.Visible : Visibility.Collapsed;
        childrenDetails.Children.Clear();
        if (model.Preview is { Children.IsEmpty: false } preview)
        {
            var counts = new[] { ("subagent.group", preview.Children.Subagents), ("branch.group", preview.Children.Branches), ("exploration.group", preview.Children.Explorations) };
            childrenDetails.Children.Add(Mute(string.Join(" · ", counts.Where(c => c.Item2 > 0).Select(c => Design.T(c.Item1) + " " + c.Item2))));
            if (!model.IncludeChildren) childrenDetails.Children.Add(Mute(Design.T(preview.Children.Explorations > 0 ? "session.children.exploration_blocks" : "session.children.keep")));
        }
        childrenDetails.Visibility = childrenDetails.Children.Count == 0 ? Visibility.Collapsed : Visibility.Visible;
        artifactDetails.Children.Clear(); artifactDetails.Visibility = model.IncludeArtifacts ? Visibility.Visible : Visibility.Collapsed;
        if (model.IncludeArtifacts)
        {
            artifactDetails.Children.Add(Mute(Design.T("session.artifacts.scope")));
            var previews = model.ArtifactPreviews.ToArray();
            if (previews.Length > 0)
            {
                var files = previews.SelectMany(p => p.Files).Distinct().Order(StringComparer.Ordinal).ToArray();
                artifactDetails.Children.Add(Design.Text(Design.T("session.artifacts.count").Replace("{artifacts}", previews.Sum(p => p.Artifacts.Length).ToString()).Replace("{files}", files.Length.ToString()), 14));
                if (files.Length > 0) artifactDetails.Children.Add(FileList(files));
                var kept = previews.SelectMany(p => p.Retained).ToArray();
                if (kept.Length > 0) { artifactDetails.Children.Add(Design.Text(Design.T("session.artifacts.retained"), 14)); artifactDetails.Children.Add(FileList(kept.Select(p => p.Name + " — " + Design.T("session.artifacts." + p.Reason)))); }
            }
        }
        loading.Visibility = model.Loading || model.Deleting ? Visibility.Visible : Visibility.Collapsed;
        var failure = model.Error ?? (model.IncludeArtifacts ? model.Preview?.ArtifactError : null);
        error.Text = model.MutationUnconfirmed ? Design.L("操作结果未能确认；后续删除已停止。关闭后刷新会话列表，操作不会自动重试。", "The result could not be confirmed. Further deletions stopped. Close to refresh the conversation list; this operation will not be retried automatically.") + "\n" + failure : failure ?? "";
        error.Visibility = failure == null ? Visibility.Collapsed : Visibility.Visible;
        retry.Visibility = failure != null && !model.MutationUnconfirmed ? Visibility.Visible : Visibility.Collapsed; retry.IsEnabled = !model.Loading;
        confirm.IsEnabled = model.CanDelete; confirm.Opacity = model.CanDelete ? 1 : .5; cancel.IsEnabled = !model.Deleting;
        Design.ApplyTypography(this);
    }
    private ScrollViewer FileList(IEnumerable<string> paths)
    {
        var rows = new StackPanel { Spacing = 4, Padding = new Thickness(18, 0, 0, 0) };
        foreach (var path in paths) { var label = Design.Text(path, 12); label.TextWrapping = TextWrapping.WrapWholeWords; rows.Children.Add(label); }
        return new ScrollViewer { Content = rows, MaxHeight = Design.FontSize(140), HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled };
    }
    public override void HandleEscape() { if (!model.Deleting) base.HandleEscape(); }
    public override void Dispose() { disposed = true; model.Changed -= Refresh; model.Dispose(); base.Dispose(); }
}
