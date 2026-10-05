using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Automation;
using Windows.System;

namespace Wisp.Science.Preview;

internal sealed partial class NativeWorkspacePanel
{
    public void CloseRunReview() { reviewNavigation++; runReview?.Close(); }
    public bool HandleEscape()
    {
        if (sideChatModelPicker?.IsDropDownOpen == true) { sideChatModelPicker.IsDropDownOpen = false; return true; }
        if (openMenus.LastOrDefault() is { } menu) { menu.Hide(); return true; }
        if (fileDialog != null) { fileDialog.Hide(); return true; }
        if (model.Tabs.Selected == "artifacts" && transcriptTables.Selected != null)
        { transcriptTables.Dismiss(); Render(); return true; }
        if (documentPreview?.DismissSelection() == true) return true;
        if (sourceSelectionEditor is { IsLoaded: true, SelectionLength: > 0 } sourceEditor) {
            sourceEditor.Select(sourceEditor.SelectionStart, 0); return true;
        }
        if (model.Preview != null) { model.DismissPreview(); Render(); return true; }
        if (model.Tabs.Selected != "hosts" || runReview?.Visible != true) return false;
        if (runReview.Confirmation != null) runReview.CancelConfirmation();
        else _ = ReturnFromRunReviewAsync();
        return true;
    }
    private async Task ReturnFromRunReviewAsync()
    {
        var runId = runReview!.RunId;
        var closing = runReview.CloseAsync();
        if (!runReview.Mutating) await PresentRunAsync(runId, true);
        await closing;
    }

    private void RenderRunReview()
    {
        var review = runReview!;
        body.Children.Add(TextHeading("运行结果与清理"));
        body.Children.Add(Mute("Run · " + review.RunId));
        if (model.RunDetail is { } detail && detail.Id == review.RunId)
        {
            body.Children.Add(Mute("执行环境 · " + detail.ContextId));
            if (!string.IsNullOrWhiteSpace(detail.RemoteWorkdir))
                body.Children.Add(Mute("服务器工作目录 · " + detail.RemoteWorkdir));
        }
        if (review.Confirmation is { } action)
        {
            body.Children.Add(Mute(action == "cleanup"
                ? "将清理此 Run 的整个服务器工作目录。尚未下载的内容将丢失；宿主会先保存运行日志。"
                : "将永久删除以下服务器文件或目录。目录中的内容也会删除："));
            foreach (var path in action == "delete" ? review.ConfirmedPaths : []) body.Children.Add(Mute(path));
            var confirm = new Button { Content = action == "cleanup" ? "确认清理整个目录" : "确认删除所选内容", IsEnabled = review.CanChange };
            confirm.Click += async (_, _) => await review.ConfirmAsync(lifetime.Token);
            var cancel = new Button { Content = "返回，不删除" };
            cancel.Click += (_, _) => review.CancelConfirmation();
            body.Children.Add(confirm); body.Children.Add(cancel); return;
        }
        var back = new Button { Content = "返回运行详情" };
        back.Click += async (_, _) => await ReturnFromRunReviewAsync(); body.Children.Add(back);
        body.Children.Add(Mute("仅下载明确选择的内容并登记为项目产物。目录会作为归档下载；浏览不会传输文件或删除内容。"));
        if (review.Mutating) body.Children.Add(Mute("正在处理。关闭视图不会撤销已提交的操作。"));
        if (review.Loading || review.Mutating) body.Children.Add(new ProgressBar { IsIndeterminate = true, Height = 3 });
        if (review.Error is { } error) body.Children.Add(new TextBlock { Text = error, TextWrapping = TextWrapping.Wrap, Foreground = design.Brush("clay-strong") });
        if (review.Status is { } status) body.Children.Add(Mute(status));
        if (review.ReadOnly && review.Ready) body.Children.Add(Mute("当前作用域只读，可浏览，不能下载或删除。"));
        if (!review.Ready && !review.Loading && !review.Mutating && review.Error == null)
            body.Children.Add(Mute("请刷新文件列表后继续。"));
        var refresh = new Button { Content = "刷新列表", IsEnabled = !review.Loading && !review.Mutating };
        refresh.Click += async (_, _) => await review.ReadAsync(false, lifetime.Token); body.Children.Add(refresh);
        if (review.Cleaned) return;
        body.Children.Add(Mute("目录：" + (review.Path.Length == 0 ? "/" : review.Path)));
        if (review.Path.Length > 0)
        {
            var up = new Button { Content = "上一级", IsEnabled = !review.Loading && !review.Mutating };
            up.Click += async (_, _) => await review.NavigateAsync(review.Parent, lifetime.Token); body.Children.Add(up);
        }
        var query = new TextBox { Text = review.Filter, PlaceholderText = "按名称筛选", IsEnabled = !review.Mutating };
        query.TextChanged += (_, _) => review.Filter = query.Text;
        query.KeyDown += async (_, e) => { if (e.Key == VirtualKey.Enter) { e.Handled = true; await review.ReadAsync(false, lifetime.Token); } };
        var search = new Button { Content = "筛选", IsEnabled = !review.Loading && !review.Mutating };
        search.Click += async (_, _) => await review.ReadAsync(false, lifetime.Token);
        body.Children.Add(query); body.Children.Add(search);
        var count = Mute("");
        var download = new Button { Content = "下载所选内容" };
        var delete = new Button { Content = "删除所选内容…" };
        void SelectionChanged()
        {
            count.Text = $"已选 {review.Selection.Count} 项（含其他目录中的选择）";
            download.IsEnabled = delete.IsEnabled = review.CanChange && review.Selection.Count > 0;
        }
        var boxes = new List<CheckBox>();
        var selectAll = new Button { Content = "全选 / 取消本页选择", IsEnabled = review.CanChange };
        selectAll.Click += (_, _) => { var select = boxes.Any(box => box.IsEnabled && box.IsChecked != true); foreach (var box in boxes.Where(box => box.IsEnabled)) box.IsChecked = select; };
        body.Children.Add(selectAll);
        foreach (var entry in review.Entries)
        {
            var row = new StackPanel { Spacing = 4 };
            var selected = new CheckBox { Content = new TextBlock { Text = entry.Path, TextWrapping = TextWrapping.Wrap }, IsChecked = review.Selection.ContainsKey(entry.Path), IsEnabled = review.CanChange && entry.Kind is "file" or "dir" };
            AutomationProperties.SetName(selected, "选择 " + entry.Path);
            selected.Checked += (_, _) => { review.Select(entry, true); SelectionChanged(); };
            selected.Unchecked += (_, _) => { review.Select(entry, false); SelectionChanged(); };
            boxes.Add(selected); row.Children.Add(selected);
            row.Children.Add(Mute(entry.Kind == "dir" ? $"目录 · {entry.FileCount?.ToString() ?? "未知"} 个文件" : $"文件 · {entry.SizeBytes:N0} 字节"));
            if (entry.Kind == "dir")
            {
                var open = new Button { Content = "打开目录", IsEnabled = !review.Loading && !review.Mutating };
                open.Click += async (_, _) => await review.NavigateAsync(entry.Path, lifetime.Token); row.Children.Add(open);
            }
            body.Children.Add(row);
        }
        if (review.Ready && review.Entries.Length == 0) body.Children.Add(Mute("此目录或筛选条件下没有文件。"));
        if (review.Truncated)
        {
            var more = new Button { Content = "加载更多", IsEnabled = !review.Loading && !review.Mutating };
            more.Click += async (_, _) => await review.ReadAsync(true, lifetime.Token); body.Children.Add(more);
        }
        SelectionChanged(); body.Children.Add(count);
        download.Click += async (_, _) => await review.DownloadAsync(lifetime.Token);
        delete.Click += (_, _) => review.BeginConfirmation("delete");
        var cleanup = new Button { Content = "清理整个工作目录…", IsEnabled = review.CanChange };
        cleanup.Click += (_, _) => review.BeginConfirmation("cleanup");
        if (!review.ReadOnly) { body.Children.Add(download); body.Children.Add(delete); body.Children.Add(cleanup); }
    }
}
