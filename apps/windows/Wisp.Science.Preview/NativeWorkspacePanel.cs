using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class NativeWorkspacePanel : UserControl, IDisposable
{
    private readonly WorkspacePanelModel model;
    private readonly WispDesign design;
    private readonly WorkspaceConversationModel? conversation;
    private readonly WorkspaceSideChatModel? sideChat;
    private readonly Func<NativeInputPreferences> inputPreferences;
    private readonly Action<string>? openTerminal;
    private readonly Action close;
    private readonly WorkspaceRunReviewModel? runReview;
    private readonly Action<string, string>? tabChanged;
    private readonly TextBox filter = new() { PlaceholderText = "筛选名称" };
    private readonly StackPanel body = new() { Spacing = 8 };
    private readonly ScrollViewer scroll = new() { HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled };
    private readonly ContentControl panelEmpty = new() { Visibility = Visibility.Collapsed,
        HorizontalContentAlignment = HorizontalAlignment.Center, VerticalContentAlignment = VerticalAlignment.Center };
    private readonly ContentControl sideChatHost = new() { Visibility = Visibility.Collapsed,
        HorizontalContentAlignment = HorizontalAlignment.Stretch, VerticalContentAlignment = VerticalAlignment.Stretch };
    private readonly Action? manageWorkflows;
    private readonly ContentControl agentsFooter = new() { Visibility = Visibility.Collapsed, HorizontalContentAlignment = HorizontalAlignment.Stretch };
    private readonly NativePanelViewState viewState = new();
    private NativePanelViewState.ReadingState? readingState;
    private bool syncingFilter;
    private bool restoreScroll;
    private readonly NativePanelTabStrip tabStrip;
    private ComboBox? sideChatModelPicker;
    private readonly List<FlyoutBase> openMenus = [];
    private ContentDialog? fileDialog;
    private NativeArtifactDialog? artifactDialog;
    private readonly Action<WorkspaceArtifactModel, string>? openCenter;
    private readonly Func<string, bool>? insertArtifactDraft;
    private readonly NativeMemosPane? memosPane;
    private readonly StackPanel panelFilters;
    private readonly Border searchCard = new();
    private readonly Dictionary<string, FrameworkElement> previewImages = [];
    private readonly HashSet<string> previewLoading = [];
    private NativeRichPreview? documentPreview;
    private NativePanelFileContent? documentContent;
    private NativePanelFileContent? scientificSource;
    private NativeSourceSelectionActions? sourceSelectionActions;
    private readonly Func<string, NativeDocumentSelection, bool>? quoteDocument;
    private readonly Func<bool>? windowEscape;
    private readonly CancellationTokenSource lifetime = new();
    private bool disposed;
    private int reviewNavigation;
    public async Task ShowFilesAsync() => await ShowTabAsync("files");
    public void ReopenTabs()
    {
        if (model.Tabs.Open.Count > 0) return;
        model.Tabs.Reopen();
        _ = ShowTabAsync(model.Tabs.Selected);
    }
    public Task ShowRunAsync(string runId) => PresentRunAsync(runId, true);
    public async Task ShowRunReviewAsync(string runId)
    {
        var loading = PresentRunAsync(runId, true);
        var current = reviewNavigation;
        await loading;
        if (!disposed && current == reviewNavigation && model.Tabs.Selected == "runs"
            && model.SelectedRunId == runId && runReview != null && model.RunDetail is { Kind: "ssh_direct" } run
            && WorkspaceConversationModel.RunTerminal(run.Status)) await runReview.OpenAsync(runId, lifetime.Token, model.Activity?.ReadOnly != false);
    }
    private async Task PresentRunAsync(string runId, bool openRuns)
    {
        if (disposed) return;
        reviewNavigation++;
        runReview?.Close();
        var read = openRuns ? model.OpenRunAsync(runId, lifetime.Token) : model.ReadRunAsync(runId, lifetime.Token);
        viewState.For("runs").SetOffset(0);
        if (openRuns) tabChanged?.Invoke(model.Tabs.Selected, model.Tabs.Saved);
        Render();
        await read;
        Render();
    }
    public async Task ShowTabAsync(string tab)
    {
        if (tab != model.Tabs.Selected) completedRunsOpen = false;
        reviewNavigation++;
        runReview?.Close();
        try
        {
            var refresh = model.RefreshAsync(tab, cancellationToken: lifetime.Token);
            tabChanged?.Invoke(model.Tabs.Selected, model.Tabs.Saved);
            Render();
            await refresh;
            Render();
        }
        catch (OperationCanceledException) { }
    }

    public NativeWorkspacePanel(WorkspacePanelModel model, WorkspaceConversationModel? conversation, WispDesign design, Action close,
        WorkspaceSideChatModel? sideChat = null, Action<string>? openTerminal = null, Action<string, string>? tabChanged = null,
        WorkspaceRunReviewModel? runReview = null, Func<NativeInputPreferences>? inputPreferences = null,
        Func<string, NativeDocumentSelection, bool>? quoteDocument = null, Func<bool>? windowEscape = null,
        Func<Task<IReadOnlyList<string>>>? pickFiles = null, Func<string, Task<string?>>? pickDownload = null,
        Action<WorkspaceArtifactModel, string>? openCenter = null, Func<string, bool>? insertArtifactDraft = null,
        WorkspaceMemosModel? memos = null, Func<string, bool>? insertMemo = null, Func<string>? projectRoot = null,
        Action? manageEnvironments = null, Func<string, Button, bool, Task>? openContextRuntimes = null, Action<NativeRun>? useRunInPublication = null,
        Action? manageWorkflows = null)
    {
        this.openCenter = openCenter; this.insertArtifactDraft = insertArtifactDraft;
        this.manageEnvironments = manageEnvironments;
        this.manageWorkflows = manageWorkflows;
        this.openContextRuntimes = openContextRuntimes;
        this.useRunInPublication = useRunInPublication;
        this.projectRoot = projectRoot ?? (() => "");
        this.pickFiles = pickFiles;
        this.pickDownload = pickDownload;
        transferTimer = DispatcherQueue.CreateTimer(); transferTimer.Interval = TimeSpan.FromSeconds(2);
        transferTimer.Tick += PollFileTransfers;
        runsTimer = DispatcherQueue.CreateTimer(); runsTimer.Interval = TimeSpan.FromSeconds(1);
        runsTimer.Tick += TickRuns; Loaded += StartRunsPolling; Unloaded += StopRunsPolling;
        this.quoteDocument = quoteDocument;
        this.windowEscape = windowEscape;
        this.model = model; this.conversation = conversation; this.design = design; this.close = close;
        if (conversation != null) conversation.Changed += RefreshTranscript;
        this.tabChanged = tabChanged;
        this.runReview = runReview;
        this.inputPreferences = inputPreferences ?? (() => new());
        if (runReview != null) runReview.Changed += Render;
        design.BindTypography(this);
        this.sideChat = sideChat; this.openTerminal = openTerminal;
        var root = new Grid { Background = design.Brush("bg-elev") };
        root.RowDefinitions.Add(new() { Height = GridLength.Auto });
        root.RowDefinitions.Add(new() { Height = GridLength.Auto });
        root.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) });
        root.RowDefinitions.Add(new() { Height = GridLength.Auto });
        InitializeGenerated(root);
        tabStrip = new(model.Tabs, design, Label, ShowTabAsync,
            () => tabChanged?.Invoke(model.Tabs.Selected, model.Tabs.Saved), close);
        tabStrip.ViewModes.Content = artifactsPane.ViewModes;
        root.Children.Add(tabStrip);
        filter.TextChanged += async (_, _) =>
        {
            if (syncingFilter || disposed) return;
            if (model.Tabs.Selected == "files" && model.RecursiveFileSearch)
            { await FileReadAsync(model.SearchFilesAsync(filter.Text, lifetime.Token)); return; }
            readingState?.SetFilter(filter.Text);
            Render();
        };
        var search = new Grid { ColumnSpacing = 8, Padding = new Thickness(10, 2, 10, 2) };
        search.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        search.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        search.Children.Add(design.Icon("search", 15));
        filter.BorderThickness = new Thickness(0);
        filter.Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        filter.Resources["TextControlBackgroundFocused"] = design.Brush("bg-sunken");
        filter.Resources["TextControlBorderBrushFocused"] = design.Brush("clay");
        filter.FontSize = design.FontSize(12);
        AutomationProperties.SetName(filter, design.L("筛选当前面板", "Filter current panel"));
        Grid.SetColumn(filter, 1); search.Children.Add(filter);
        searchCard.Child = search; searchCard.CornerRadius = new CornerRadius(8); searchCard.Background = design.Brush("bg-sunken");
        var filters = new StackPanel { Spacing = 12, Margin = new Thickness(20, 16, 20, 16) }; filters.Children.Add(fileHeader); filters.Children.Add(searchCard);
        panelFilters = filters;
        Grid.SetRow(filters, 1); root.Children.Add(filters);
        EnableFileDrop(root);
        scroll.Content = body; scroll.Margin = new Thickness(20, 0, 20, 12);
        scroll.ViewChanged += (_, _) =>
        {
            if (!disposed && !restoreScroll && !model.Loading) readingState?.SetOffset(scroll.VerticalOffset);
        };
        scroll.LayoutUpdated += RestoreScroll;
        Grid.SetRow(scroll, 2); root.Children.Add(scroll);
        Grid.SetRow(runsEmpty, 2); root.Children.Add(runsEmpty);
        Grid.SetRow(panelEmpty, 2); root.Children.Add(panelEmpty);
        Grid.SetRow(sideChatHost, 1); Grid.SetRowSpan(sideChatHost, 2); root.Children.Add(sideChatHost);
        Grid.SetRow(agentsFooter, 3); root.Children.Add(agentsFooter);
        if (memos != null)
        {
            memosPane = new(memos, design, insertMemo); Grid.SetRow(memosPane, 2); root.Children.Add(memosPane);
        }
        InitializeLauncher(root);
        Content = root;
        _ = StartAsync();
    }

    private async Task StartAsync()
    {
        try
        {
            var refresh = model.RefreshAsync(model.Tabs.Selected, cancellationToken: lifetime.Token);
            Render();
            await refresh;
            if (sideChat is not null) await sideChat.LoadOptionsAsync(lifetime.Token);
            Render();
        }
        catch (OperationCanceledException) { }
    }

    public void Refresh() { if (!disposed) Render(); }

    private ConversationSnapshot? Transcript => conversation?.ShowingHistory == true ? conversation.History : conversation?.Snapshot;
    private void RefreshTranscript()
    {
        if (!disposed && UpdateGenerated() && model.Tabs.Selected is not ("runs" or "hosts")) Render();
    }

    private void RestoreScroll(object? sender, object e)
    {
        if (disposed || !restoreScroll || model.Loading || readingState is null) return;
        scroll.ChangeView(null, Math.Min(readingState.Offset, scroll.ScrollableHeight), null, disableAnimation: true);
        restoreScroll = false;
    }

    private void Render()
    {
        if (disposed) return;
        runsEmpty.Visibility = Visibility.Collapsed;
        panelEmpty.Visibility = sideChatHost.Visibility = agentsFooter.Visibility = Visibility.Collapsed;
        sourceSelectionActions?.Dispose(); sourceSelectionActions = null;
        var showingLauncher = model.Tabs.Selected == NativePanelTabs.Home;
        launcherHost.Visibility = showingLauncher ? Visibility.Visible : Visibility.Collapsed;
        tabStrip.Visibility = showingLauncher ? Visibility.Collapsed : Visibility.Visible;
        if (showingLauncher)
        {
            generatedHost.Visibility = panelFilters.Visibility = scroll.Visibility = Visibility.Collapsed;
            if (memosPane != null) memosPane.Visibility = Visibility.Collapsed;
            RenderLauncher(); return;
        }
        var showingGenerated = model.Tabs.Selected == "artifacts";
        tabStrip.ViewModes.Visibility = showingGenerated ? Visibility.Visible : Visibility.Collapsed;
        generatedHost.Visibility = showingGenerated ? Visibility.Visible : Visibility.Collapsed;
        if (showingGenerated)
        {
            UpdateGenerated();
            tabStrip.Refresh(); panelFilters.Visibility = scroll.Visibility = Visibility.Collapsed;
            if (memosPane != null) memosPane.Visibility = Visibility.Collapsed;
            return;
        }
        var showingMemos = model.Tabs.Selected == "memos" && memosPane != null;
        tabStrip.Refresh();
        panelFilters.Visibility = scroll.Visibility = showingMemos ? Visibility.Collapsed : Visibility.Visible;
        if (memosPane != null) memosPane.Visibility = showingMemos ? Visibility.Visible : Visibility.Collapsed;
        if (showingMemos) { memosPane!.Activate(); return; }
        if (model.Tabs.Selected == "sidechat")
        {
            panelFilters.Visibility = scroll.Visibility = Visibility.Collapsed;
            sideChatHost.Visibility = Visibility.Visible; RenderSideChat(); design.ApplyTypography(this); return;
        }
        restoreScroll = true;
        readingState = viewState.For(model.Tabs.Selected, model.FileContext + "|" + model.Path);
        syncingFilter = true;
        filter.Text = model.Tabs.Selected == "files" && model.RecursiveFileSearch ? model.FileSearchQuery : readingState.Filter;
        syncingFilter = false;
        filter.PlaceholderText = model.Tabs.Selected == "files" ? design.T("files.search") : design.L("筛选名称", "Filter by name");
        AutomationProperties.SetName(filter, filter.PlaceholderText);
        body.Spacing = model.Tabs.Selected == "files" ? 2 : 8;
        body.Children.Clear();
        fileHeader.Children.Clear(); fileHeader.Visibility = model.Tabs.Selected == "files" ? Visibility.Visible : Visibility.Collapsed;
        sideChatModelPicker = null;
        searchCard.Visibility = model.Tabs.Selected is "hosts" or "runs" or "sidechat" || model.Tabs.Selected == "files" && model.RemoteFiles ? Visibility.Collapsed : Visibility.Visible;
        if (model.HasPendingFileTransfers && model.FileTransferReadError == null) transferTimer.Start(); else transferTimer.Stop();
        var runsTab = model.Tabs.Selected == "runs";
        var hostsTab = model.Tabs.Selected == "hosts";
        panelFilters.Visibility = runsTab || hostsTab || model.Tabs.Selected == "agents" ? Visibility.Collapsed : Visibility.Visible;
        scroll.Margin = runsTab || hostsTab ? new Thickness(12) : new Thickness(20, 0, 20, 12);
        scroll.VerticalContentAlignment = VerticalAlignment.Stretch;
        body.HorizontalAlignment = HorizontalAlignment.Stretch;
        body.Spacing = runsTab ? 14 : body.Spacing;
        if (Content is Grid panelRoot) panelRoot.Background = design.Brush(hostsTab || model.Tabs.Selected == "agents" || runsTab && !model.SessionRuns.Empty ? "bg-sunken" : "bg-elev");
        if (runReview?.Visible == true && runsTab)
        { RenderRunReview(); design.ApplyTypography(this); return; }
        if (model.Loading) body.Children.Add(new ProgressBar { IsIndeterminate = true, Height = 3 });
        if (model.Error is { } error) body.Children.Add(new TextBlock { Text = error, TextWrapping = TextWrapping.Wrap, Foreground = design.Brush("clay-strong"), FontSize = 12 });
        var query = filter.Text.Trim();
        if (model.Tabs.Selected == "files") RenderFiles(query);
        else if (model.Tabs.Selected == "hosts") RenderHosts();
        else if (runsTab) RenderRuns();
        else if (model.Tabs.Selected == "agents") RenderAgents(query);
        else if (model.Tabs.Selected == "notebook") RenderNotebook(query);
        else if (model.Tabs.Selected == "highlights") RenderHighlights(query);
        else if (model.Tabs.Selected == "provenance")
        {
            foreach (var row in NativeProvenanceRow.Collect(Transcript?.Items ?? []).Where(item => item.Matches(query)))
            {
                var detail = new StackPanel { Spacing = 8 };
                foreach (var (label, value) in new[] { (design.L("输入", "Input"), row.Input), (design.L("输出", "Output"), row.Output) })
                {
                    if (value.Length == 0) continue;
                    detail.Children.Add(Mute(label));
                    detail.Children.Add(new TextBox { AcceptsReturn = true, Text = value, IsReadOnly = true,
                        TextWrapping = TextWrapping.Wrap, MaxHeight = 240, FontFamily = design.Font(true), FontSize = design.FontSize(12, true) });
                }
                var disclosure = new Expander { Header = design.Text(row.Name, 13), Content = detail,
                    HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch };
                AutomationProperties.SetName(disclosure, row.Name); body.Children.Add(disclosure);
            }
        }
        if (model.Tabs.Selected is not ("files" or "runs") && body.Children.Count == 0 && !model.Loading && model.Error == null)
        {
            panelEmpty.Content = new NativePanelEmptyState(design, model.Tabs.Selected, query.Length > 0);
            panelEmpty.Visibility = Visibility.Visible; scroll.Visibility = Visibility.Collapsed;
            if (query.Length == 0) panelFilters.Visibility = Visibility.Collapsed;
        }
        RenderPreview();
        design.ApplyTypography(this);
    }

    private void TrackMenu(FlyoutBase menu)
    {
        menu.Opened += (_, _) => { openMenus.Remove(menu); openMenus.Add(menu); };
        menu.Closed += (_, _) => openMenus.Remove(menu);
    }

    private async Task PromptFileAsync(NativePanelFileAction action, string currentName = "")
    {
        if (!model.CanMutateLocalFiles) return;
        var directory = model.Path; var context = model.FileContext;
        var name = new TextBox { Text = currentName, PlaceholderText = design.T("files.name") };
        var dialog = new ContentDialog
        {
            Title = action switch { NativePanelFileAction.CreateFile => design.T("files.new_file"), NativePanelFileAction.CreateDirectory => design.T("files.new_directory"), NativePanelFileAction.Rename => design.T("files.rename"), _ => design.L("删除", "Delete") },
            Content = action == NativePanelFileAction.Delete ? new TextBlock { Text = design.T("files.delete_directory_confirm").Replace("{path}", currentName), TextWrapping = TextWrapping.Wrap } : name,
            PrimaryButtonText = action == NativePanelFileAction.Delete ? design.L("删除", "Delete") : design.L("确定", "Confirm"),
            CloseButtonText = design.L("取消", "Cancel"),
            XamlRoot = XamlRoot
        };
        fileDialog = dialog;
        ContentDialogResult result;
        try { result = await dialog.ShowAsync(); }
        finally { fileDialog = null; }
        if (result != ContentDialogResult.Primary || disposed || model.Path != directory || model.FileContext != context) return;
        try
        {
            var target = NativePanelPaths.Destination(directory, action is NativePanelFileAction.Rename or NativePanelFileAction.Delete ? currentName : name.Text);
            var destination = action == NativePanelFileAction.Rename ? NativePanelPaths.Destination(directory, name.Text) : null;
            await model.PerformFileActionAsync(action, target, destination, lifetime.Token);
            Render();
        }
        catch (Exception ex) { await ShowErrorAsync("操作未确认成功，请刷新核对后再操作。\n" + ex.Message); Render(); }
    }

    private async Task ShowErrorAsync(string message)
    {
        var dialog = new ContentDialog { Title = design.L("文件操作", "File operation"), Content = message, CloseButtonText = design.L("关闭", "Close"), XamlRoot = XamlRoot };
        fileDialog = dialog;
        try { await dialog.ShowAsync(); }
        finally { fileDialog = null; }
    }

    private void RenderRunDetail()
    {
        if (model.SelectedRunId is not { } selected) return;
        body.Children.Add(TextHeading(design.L("运行详情", "Run details") + " · " + selected));
        if (runReview?.DismissalError is { } dismissalError) body.Children.Add(Mute(dismissalError));
        if (runReview?.Mutating == true) body.Children.Add(Mute(design.L("审阅操作仍在处理，详情可能尚未更新。", "A review action is still in progress; details may be outdated.")));
        var actions = new NativeActionWrap();
        var refresh = new Button { Content = design.L("刷新详情", "Refresh details"), IsEnabled = !model.RunLoading };
        refresh.Click += async (_, _) => await PresentRunAsync(selected, false);
        var dismiss = new Button { Content = design.L("关闭详情", "Close details") };
        dismiss.Click += (_, _) => { model.DismissRun(); Render(); };
        actions.Children.Add(refresh); actions.Children.Add(dismiss); body.Children.Add(actions);
        if (model.RunLoading) body.Children.Add(new ProgressBar { IsIndeterminate = true, Height = 3 });
        if (model.RunError is { } error)
            body.Children.Add(new TextBlock { Text = error + (model.RunDetail != null ? design.L("\n以下为上次读取的详情。", "\nShowing the last available details.") : ""),
                TextWrapping = TextWrapping.Wrap, Foreground = design.Brush("clay-strong") });
        if (model.RunDetail is { } runDetail)
        {
            var card = new StackPanel { Spacing = 4 };
            card.Children.Add(TextHeading(runDetail.Title));
            card.Children.Add(Mute($"{NativeToolPresentation.RunState(runDetail.Status, design.Localization)} · {runDetail.Kind} · {runDetail.ContextId}"));
            if (runReview != null && model.Activity?.RunReviewSupported == true && runDetail.Kind == "ssh_direct"
                && runDetail.Status is "succeeded" or "failed" or "cancelled" or "timed_out" or "lost")
            {
                var review = new Button { Content = design.L("结果与清理", "Results and cleanup"), IsEnabled = !model.ActivityBusy && !runReview.Mutating };
                review.Click += async (_, _) => { viewState.For("runs").SetOffset(0); await runReview.OpenAsync(runDetail.Id, lifetime.Token, model.Activity.ReadOnly); };
                card.Children.Add(review);
            }
            if (runDetail.ExitCode is { } exit) card.Children.Add(Mute(design.L("退出码：", "Exit code: ") + exit));
            if (!string.IsNullOrEmpty(runDetail.RemoteWorkdir)) card.Children.Add(Mute(design.L("工作目录：", "Working directory: ") + runDetail.RemoteWorkdir));
            if (!string.IsNullOrEmpty(runDetail.Command)) card.Children.Add(new TextBox { Text = runDetail.Command, IsReadOnly = true, TextWrapping = TextWrapping.Wrap, FontFamily = design.Font(true), FontSize = design.FontSize(11, true) });
            if (!string.IsNullOrEmpty(runDetail.StdoutTail))
            {
                card.Children.Add(Mute(design.L("标准输出（末尾片段）", "Standard output (tail)")));
                card.Children.Add(new TextBox { AcceptsReturn = true, Text = runDetail.StdoutTail, IsReadOnly = true, TextWrapping = TextWrapping.Wrap, FontFamily = design.Font(true), FontSize = design.FontSize(11, true), MaxHeight = 180 });
            }
            if (!string.IsNullOrEmpty(runDetail.StderrTail))
            {
                card.Children.Add(Mute(design.L("标准错误（末尾片段）", "Standard error (tail)")));
                card.Children.Add(new TextBox { AcceptsReturn = true, Text = runDetail.StderrTail, IsReadOnly = true, TextWrapping = TextWrapping.Wrap, FontFamily = design.Font(true), FontSize = design.FontSize(11, true), MaxHeight = 120 });
            }
            if (string.IsNullOrEmpty(runDetail.StdoutTail) && string.IsNullOrEmpty(runDetail.StderrTail))
                card.Children.Add(Mute(design.L("本次读取暂无输出。", "No output available yet.")));
            if (!string.IsNullOrEmpty(runDetail.LastPollError)) card.Children.Add(Mute(design.L("上次状态轮询失败：", "Last status poll failed: ") + runDetail.LastPollError));
            if (runDetail.CleanupError is { } cleanupError) card.Children.Add(Mute(cleanupError));
            body.Children.Add(card);
        }
    }

    private TextBlock TextHeading(string value) => new()
    {
        Text = value, FontSize = 12, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold,
        Foreground = design.Brush("text-muted"), Margin = new Thickness(0, 10, 0, 0)
    };

    private void RenderAgents(string query)
    {
        if (manageWorkflows != null)
        {
            var manage = design.ToolButton(design.T("agents.manage_workflows"), "branch");
            var label = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
            label.Children.Add(design.Icon("branch", 14)); label.Children.Add(design.Text(design.T("agents.manage_workflows"), 12));
            manage.Content = label; manage.HorizontalAlignment = HorizontalAlignment.Stretch;
            manage.Click += (_, _) => manageWorkflows();
            agentsFooter.Content = new Border { Child = manage, Padding = new(12, 8, 12, 8), BorderThickness = new(0, 1, 0, 0), BorderBrush = design.Brush("border") };
            agentsFooter.Visibility = Visibility.Visible;
        }
        if (model.Agents.Length == 0)
        {
            var empty = design.Text(design.T("agents.activity.empty"), 12, "text-faint");
            empty.MaxWidth = 260; empty.TextAlignment = TextAlignment.Center; empty.HorizontalAlignment = HorizontalAlignment.Center;
            empty.Margin = new(12, 24, 12, 24); body.Children.Add(empty); return;
        }
        // Explanation card mirrors the macOS alignment: delegation is opt-in and
        // never implies auto-approval; approval actions below stay version-checked.
        var intro = new StackPanel { Spacing = 4, Padding = new Thickness(10), Background = design.Brush("bg-elev"), CornerRadius = new CornerRadius(8) };
        intro.Children.Add(new TextBlock { Text = design.L("Agent 工作流", "Agent workflows"), FontSize = 13, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold });
        intro.Children.Add(new TextBlock
        {
            Text = design.L("工作流由会话中的 Agent 创建。需要确认的工作流必须先批准；开启委派不会自动批准任何操作。", "The conversation's agent creates workflows. Workflows requiring confirmation must be approved first; delegation does not automatically approve operations."),
            FontSize = 11, Foreground = design.Brush("text-muted"), TextWrapping = TextWrapping.Wrap
        });
        body.Children.Add(intro);
        if (model.DelegationEnabled is { } delegation)
        {
            var delegationCard = new StackPanel { Spacing = 4, Padding = new Thickness(10), Background = design.Brush(delegation ? "bg-elev" : "bg-sunken"), CornerRadius = new CornerRadius(8) };
            delegationCard.Children.Add(new TextBlock
            {
                Text = delegation ? design.L("委派已开启：会话可把子任务委派给工作流。", "Delegation is on: this conversation can delegate subtasks to workflows.") : design.L("委派已关闭：会话不会创建新的委派任务。", "Delegation is off: this conversation will not create new delegated tasks."),
                FontSize = 12, TextWrapping = TextWrapping.Wrap
            });
            var toggle = new Button
            {
                Content = delegation ? design.L("关闭委派", "Turn off delegation") : design.L("开启委派", "Turn on delegation"),
                IsEnabled = !model.DelegationBusy && model.Tabs.Selected == "agents"
            };
            toggle.Click += async (_, _) => { await model.SetDelegationAsync(!delegation, lifetime.Token); Render(); };
            delegationCard.Children.Add(toggle);
            body.Children.Add(delegationCard);
        }
        foreach (var agent in model.Agents.Where(item => query.Length == 0 || item.Workflow.Name.Contains(query, StringComparison.CurrentCultureIgnoreCase)))
        {
            var captured = agent;
            var card = new StackPanel { Spacing = 4 };
            card.Children.Add(Row(captured.Workflow.Name, captured.Workflow.Status + (captured.Workflow.Depth > 0 ? design.L(" · 子工作流", " · Subworkflow") : ""), "sparkles"));
            if (captured.Workflow.Depth == 0)
            {
                var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4 };
                foreach (var (label, action) in new (string, NativeAgentAction)[] { (design.L("批准", "Approve"), NativeAgentAction.Approve), (design.L("运行", "Run"), NativeAgentAction.Run), (design.L("取消", "Cancel"), NativeAgentAction.Cancel) })
                {
                    var button = new Button { Content = label };
                    button.Click += async (_, _) => { await model.ActAgentAsync(captured, action, lifetime.Token); Render(); };
                    row.Children.Add(button);
                }
                card.Children.Add(row);
            }
            body.Children.Add(card);
        }
    }

    private void RenderNotebook(string query)
    {
        foreach (var cell in NativeNotebookCell.Collect(Transcript?.Items ?? []).Where(cell => query.Length == 0 || cell.Source.Contains(query, StringComparison.CurrentCultureIgnoreCase)))
        {
            var captured = cell;
            var card = new StackPanel { Spacing = 10, Padding = new Thickness(12), Background = design.Brush("bg-sunken"), CornerRadius = new CornerRadius(10) };
            var heading = design.Text(captured.Language + " · " + StatusLabel(cell), 12);
            heading.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; card.Children.Add(heading);
            card.Children.Add(new TextBox
            {
                AcceptsReturn = true, Text = captured.Source, IsReadOnly = true, TextWrapping = TextWrapping.Wrap,
                FontFamily = design.Font(true), FontSize = design.FontSize(12, true), MaxHeight = 160,
                BorderThickness = new Thickness(0), Background = design.Brush("bg-elev")
            });
            if (captured.Output.Length > 0)
                card.Children.Add(new Expander
                {
                    Header = new TextBlock { Text = design.L("输出", "Output"), FontSize = 11, Foreground = design.Brush("text-muted") },
                    Content = new TextBox
                    {
                        AcceptsReturn = true, Text = captured.Output, IsReadOnly = true, TextWrapping = TextWrapping.Wrap,
                        FontFamily = design.Font(true), FontSize = design.FontSize(11, true), MaxHeight = 220
                    },
                    IsExpanded = cell.OutputInitiallyExpanded
                });
            var star = new Button { Content = model.NotebookStars.Any(item => item.Matches(captured)) ? design.L("取消收藏", "Unsave code") : design.L("收藏代码", "Save code") };
            design.QuietButton(star);
            star.Click += async (_, _) => { await model.ToggleNotebookStarAsync(captured, lifetime.Token); Render(); };
            card.Children.Add(star); body.Children.Add(card);
        }
    }

    private string StatusLabel(NativeNotebookCell cell) => cell.Status switch
    {
        "ok" => design.L("执行完成", "Completed"), "error" => design.L("执行出错", "Failed"), "source" => design.L("草稿", "Draft"), _ => design.L("运行中", "Running")
    };

    private void RenderHighlights(string query)
    {
        foreach (var row in model.Highlights.Where(item => query.Length == 0 || item.Code.Contains(query, StringComparison.CurrentCultureIgnoreCase)))
        {
            var captured = row;
            var card = new StackPanel { Spacing = 4 };
            card.Children.Add(Row(captured.Title, captured.Code, "book"));
            var remove = new Button { Content = design.L("移除", "Remove") };
            remove.Click += async (_, _) => { await model.RemoveHighlightAsync(captured.Id, lifetime.Token); Render(); };
            card.Children.Add(remove); body.Children.Add(card);
        }
    }

    private void RenderSideChat()
    {
        var root = new Grid { Background = design.Brush("bg-elev") };
        root.RowDefinitions.Add(new() { Height = GridLength.Auto }); root.RowDefinitions.Add(new()); root.RowDefinitions.Add(new() { Height = GridLength.Auto });
        sideChatHost.Content = root;
        var header = new Grid { Padding = new(14, 8, 10, 8) };
        header.ColumnDefinitions.Add(new()); header.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var title = design.Text(design.T("sidechat.title"), 13); title.VerticalAlignment = VerticalAlignment.Center; header.Children.Add(title);
        var dismiss = design.ToolButton(design.T("sidechat.close"), "close");
        dismiss.Click += async (_, _) =>
        {
            model.Tabs.Remove("sidechat");
            await ShowTabAsync(model.Tabs.Open.Count == 0 ? NativePanelTabs.Home : model.Tabs.Selected);
        };
        Grid.SetColumn(dismiss, 1); header.Children.Add(dismiss); root.Children.Add(header);
        if (sideChat is null)
        {
            var unavailable = Mute(design.L("侧聊尚未连接", "Side chat is not connected yet."));
            unavailable.Margin = new(18); Grid.SetRow(unavailable, 1); root.Children.Add(unavailable); return;
        }
        var history = new StackPanel { Spacing = 14, Padding = new(14) };
        var messages = new ScrollViewer { Content = history, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled };
        Grid.SetRow(messages, 1); root.Children.Add(messages);
        if (sideChat.Rows.Count == 0)
        {
            messages.Visibility = Visibility.Collapsed;
            var empty = Mute(design.T("sidechat.empty")); empty.TextAlignment = TextAlignment.Center; empty.MaxWidth = 280;
            empty.HorizontalAlignment = HorizontalAlignment.Center; empty.VerticalAlignment = VerticalAlignment.Center;
            Grid.SetRow(empty, 1); root.Children.Add(empty);
        }
        foreach (var row in sideChat.Rows)
        {
            var question = design.Text(row.Question, 13); question.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; history.Children.Add(question);
            history.Children.Add(design.Text(row.Error ?? row.Answer?.Answer ?? "…", 13, row.Error == null ? "text" : "clay-strong"));
            if (row.Model is { Length: > 0 } label) history.Children.Add(design.Text(label, 11, "text-faint"));
        }
        if (sideChat.Busy) history.Children.Add(Mute(design.L("正在回复…", "Replying…")));
        var composer = new StackPanel { Spacing = 8, Padding = new(14, 12, 12, 10) };
        var picker = new ComboBox { HorizontalAlignment = HorizontalAlignment.Left, MinWidth = 0, MaxWidth = 250,
            CornerRadius = new(16), FontSize = 12, IsEnabled = !sideChat.Busy && !sideChat.ChangingModel };
        sideChatModelPicker = picker;
        foreach (var option in sideChat.Options) picker.Items.Add(new ComboBoxItem { Content = option.Label, Tag = option });
        picker.SelectedItem = picker.Items.Cast<ComboBoxItem>().FirstOrDefault(item => ((NativeSideChatOption)item.Tag).Key == sideChat.Selected?.Key);
        AutomationProperties.SetName(picker, design.L("侧聊模型", "Side chat model"));
        AutomationProperties.SetAutomationId(picker, "sidechat-model-trigger");
        picker.SelectionChanged += async (_, _) =>
        {
            if (picker.SelectedItem is not ComboBoxItem { Tag: NativeSideChatOption selected }) return;
            var change = sideChat.SelectAsync(selected, lifetime.Token); Render(); await change; Render();
        };
        var draft = new TextBox { AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, Text = sideChat.Draft,
            PlaceholderText = design.T("sidechat.placeholder"), MinHeight = 50, MaxHeight = 180, BorderThickness = new(0), Padding = new(0, 4, 0, 4), IsEnabled = !sideChat.Busy };
        AutomationProperties.SetName(draft, design.T("sidechat.placeholder"));
        var send = design.ToolButton(design.L("发送", "Send"), "arrow-up"); send.IsEnabled = sideChat.CanSend;
        design.ActionButton(send, true);
        send.Width = send.Height = 30; send.MinWidth = send.MinHeight = 0; send.Padding = new(6); send.CornerRadius = new(15);
        draft.TextChanged += (_, _) => { sideChat.Draft = draft.Text; send.IsEnabled = sideChat.CanSend; };
        var composing = false;
        var compositionJustEnded = false;
        draft.TextCompositionStarted += (_, _) => composing = true;
        draft.TextCompositionEnded += (_, _) =>
        {
            composing = false; compositionJustEnded = true;
            DispatcherQueue.TryEnqueue(() => compositionJustEnded = false);
        };
        draft.PreviewKeyDown += async (_, e) =>
        {
            if (e.Key != Windows.System.VirtualKey.Enter) return;
            var shift = (Microsoft.UI.Input.InputKeyboardSource.GetKeyStateForCurrentThread(Windows.System.VirtualKey.Shift) & Windows.UI.Core.CoreVirtualKeyStates.Down) != 0;
            var control = (Microsoft.UI.Input.InputKeyboardSource.GetKeyStateForCurrentThread(Windows.System.VirtualKey.Control) & Windows.UI.Core.CoreVirtualKeyStates.Down) != 0;
            if (NativeSideChatKeyboard.ResolveReturn(shift, composing || compositionJustEnded, control, inputPreferences().SendWithModifier) != NativeSideChatReturnAction.Send) return;
            e.Handled = true; await SendSideChatAsync();
        };
        send.Click += async (_, _) => await SendSideChatAsync();
        composer.Children.Add(draft);
        var actions = new Grid { ColumnSpacing = 8 }; actions.ColumnDefinitions.Add(new()); actions.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        actions.Children.Add(picker); Grid.SetColumn(send, 1); actions.Children.Add(send); composer.Children.Add(actions);
        if (sideChat.Error is { } error) composer.Children.Add(design.Text(error, 12, "clay-strong"));
        var footer = new Border { Padding = new(14, 12, 14, 12), Background = design.Brush("bg-app"),
            Child = new Border { Child = composer, BorderBrush = design.Brush("border"), BorderThickness = new(1), CornerRadius = new(16), Background = design.Brush("bg-input") } };
        Grid.SetRow(footer, 2); root.Children.Add(footer);
    }

    private async Task SendSideChatAsync()
    {
        if (disposed || sideChat?.CanSend != true) return;
        var sending = sideChat.SendAsync(lifetime.Token); Render();
        await sending; Render();
    }

    public async Task OpenSearchArtifactAsync(string id)
    {
        if (disposed) return;
        await model.RefreshAsync("artifacts", cancellationToken: lifetime.Token);
        if (!disposed) { Render(); await OpenArtifactDialogAsync(id); }
    }

    private async Task OpenArtifactDialogAsync(string id, WorkspaceArtifactModel? source = null)
    {
        if (disposed || artifactDialog != null) { source?.Close(); return; }
        var reader = source ?? model.CreateArtifactPreview();
        var dialog = artifactDialog = new NativeArtifactDialog(reader, design, XamlRoot, pickDownload, insertArtifactDraft, openCenter != null,
            quoteDocument == null ? null : request => !disposed && IsLoaded && conversation?.Snapshot is { } snapshot
                && snapshot.ProjectId == request.ProjectId && snapshot.SessionId == request.SessionId
                && quoteDocument(request.Path, request.Selection));
        try
        {
            await dialog.ShowAsync(id);
            if (!disposed && dialog.CenterRequestedId is { } selected) openCenter?.Invoke(reader.Fork(), selected);
            if (!disposed && dialog.InputRequestedPath is { } input)
            {
                syncingFilter = true; filter.Text = ""; syncingFilter = false;
                await model.RevealArtifactInputAsync(input, lifetime.Token);
                if (!disposed) { tabChanged?.Invoke(model.Tabs.Selected, model.Tabs.Saved); Render(); }
            }
        }
        finally { if (artifactDialog == dialog) artifactDialog = null; dialog.Dispose(); }
    }

    private void RenderPreview()
    {
        var openingDocument = documentContent != model.Preview;
        if (documentContent != model.Preview) { documentPreview?.Dispose(); documentPreview = null; documentContent = null; }
        if (!ReferenceEquals(scientificSource, model.Preview)) scientificSource = null;
        var scientificKind = NativeScientificPreview.Kind(model.Preview?.Path);
        if (model.Preview is { } preview && (NativeDocumentPreview.Kind(preview.Mime) ?? scientificKind) is { } kind
            && !ReferenceEquals(scientificSource, preview))
        {
            var sourceLabel = Mute(preview.Path);
            body.Children.Add(sourceLabel);
            var previewActions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
            if (scientificKind != null && preview.Text != null)
            {
                var source = new Button { Content = design.L("查看源文本", "View source") };
                source.Click += (_, _) => { scientificSource = preview; documentPreview?.Dispose(); documentPreview = null; Render(); };
                previewActions.Children.Add(source);
            }
            var dismissDocument = new Button { Content = design.L("关闭预览", "Close preview") };
            dismissDocument.Click += (_, _) => { model.DismissPreview(); Render(); };
            previewActions.Children.Add(dismissDocument);
            body.Children.Add(previewActions);
            var value = scientificKind != null ? preview.Text : preview.Base64;
            if (preview.Truncated || string.IsNullOrEmpty(value)) body.Children.Add(Mute("文档内容不完整，无法预览。请在项目文件中打开原文件。"));
            else
            {
                documentContent = preview;
                documentPreview ??= new NativeRichPreview(design, kind, value, preview.Path, quote: NativeDocumentPreview.Kind(preview.Mime) == null || quoteDocument == null ? null : selection =>
                    !disposed && IsLoaded && ReferenceEquals(model.Preview, preview) && ReferenceEquals(documentContent, preview)
                        && quoteDocument(preview.Path, selection), escape: () =>
                        {
                            if (!disposed && IsLoaded && ReferenceEquals(model.Preview, preview))
                            {
                                if (windowEscape != null) windowEscape(); else HandleEscape();
                            }
                        }, format: NativeScientificPreview.Format(preview.Path));
                body.Children.Add(documentPreview);
            }
            // The file list can be taller than the panel. Reveal a newly opened
            // document after layout, but never move the reader during refreshes.
            if (openingDocument) DispatcherQueue.TryEnqueue(Microsoft.UI.Dispatching.DispatcherQueuePriority.Low, () =>
            {
                if (!disposed && IsLoaded && sourceLabel.IsLoaded && ReferenceEquals(model.Preview, preview))
                    sourceLabel.StartBringIntoView(new BringIntoViewOptions { AnimationDesired = false, VerticalAlignmentRatio = 0 });
            });
            return;
        }
        if (model.Preview?.Text is { } text)
        {
            var sourceContent = model.Preview;
            body.Children.Add(new TextBlock { Text = model.Preview.Path, FontSize = 12, Foreground = design.Brush("text-muted") });
            var editor = new TextBox { AcceptsReturn = true, Text = text, IsReadOnly = !model.PreviewEditable, TextWrapping = TextWrapping.Wrap, MinHeight = 120 };
            body.Children.Add(editor);
            if (quoteDocument != null)
                sourceSelectionActions = new(editor, text, design,
                    () => !disposed && IsLoaded && ReferenceEquals(model.Preview, sourceContent),
                    selection => quoteDocument(sourceContent.Path, selection));
            var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
            if (scientificKind != null)
            {
                var interactive = new Button { Content = design.L("返回交互预览", "Back to preview") };
                interactive.Click += (_, _) => { scientificSource = null; documentContent = null; Render(); };
                row.Children.Add(interactive);
            }
            if (!model.PreviewEditable && model.CanEditPreview)
            {
                var edit = new Button { Content = design.L("编辑", "Edit") }; edit.Click += (_, _) => { model.EditPreview(); Render(); };
                row.Children.Add(edit);
            }
            else if (model.PreviewEditable)
            {
                var save = new Button { Content = design.L("保存", "Save") };
                save.Click += async (_, _) =>
                {
                    try { await model.SaveFileAsync(text, editor.Text, lifetime.Token); Render(); }
                    catch (Exception ex) { await ShowErrorAsync("保存未确认成功，不会自动重试。\n" + ex.Message); }
                };
                row.Children.Add(save);
            }
            var dismiss = new Button { Content = design.L("关闭预览", "Close preview") }; dismiss.Click += (_, _) => { model.DismissPreview(); Render(); };
            row.Children.Add(dismiss); body.Children.Add(row);
            return;
        }
        // Read-only image preview for artifacts/files the host returned as base64 bytes.
        var content = model.Preview;
        if (content?.Base64 is not { Length: > 0 } || content.Mime?.StartsWith("image/", StringComparison.Ordinal) != true) return;
        var key = content.Path + "#" + content.TotalBytes;
        if (previewImages.TryGetValue(key, out var loaded)) body.Children.Add(loaded);
        else if (previewLoading.Add(key)) _ = LoadPreviewImageAsync(key, content);
    }

    private async Task LoadPreviewImageAsync(string key, NativePanelFileContent content)
    {
        try
        {
            var bytes = Convert.FromBase64String(content.Base64 ?? "");
            if (bytes.Length > 0)
            {
                var source = new BitmapImage();
                using (var stream = new Windows.Storage.Streams.InMemoryRandomAccessStream())
                using (var writer = new Windows.Storage.Streams.DataWriter(stream.GetOutputStreamAt(0)))
                {
                    writer.WriteBytes(bytes);
                    await writer.StoreAsync();
                    await writer.FlushAsync();
                    stream.Seek(0);
                    await source.SetSourceAsync(stream);
                }
                var stack = new StackPanel { Spacing = 4 };
                stack.Children.Add(new TextBlock { Text = content.Path, FontSize = 12, Foreground = design.Brush("text-muted"), TextWrapping = TextWrapping.Wrap });
                stack.Children.Add(new Border
                {
                    Child = new Image { Source = source, MaxHeight = 300, Stretch = Stretch.Uniform, HorizontalAlignment = HorizontalAlignment.Left },
                    CornerRadius = new CornerRadius(8), Padding = new Thickness(4), Background = design.Brush("bg-app")
                });
                if (content.Truncated) stack.Children.Add(Mute("内容被截断；预览可能不完整。"));
                var dismiss = new Button { Content = design.L("关闭预览", "Close preview") }; dismiss.Click += (_, _) => { model.DismissPreview(); Render(); };
                stack.Children.Add(dismiss);
                previewImages[key] = stack;
            }
        }
        catch { } // Broken or non-image bytes fall back to no preview; rows stay selectable.
        finally { previewLoading.Remove(key); if (!disposed) Render(); }
    }

    private Button Row(string title, string detail, string icon)
    {
        var content = new StackPanel { Spacing = 6 };
        var heading = new Grid { ColumnSpacing = 8 };
        heading.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        heading.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        heading.Children.Add(design.Icon(icon, 14));
        var label = design.Text(title, 13); label.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        label.MaxLines = 2; label.TextTrimming = TextTrimming.CharacterEllipsis;
        Grid.SetColumn(label, 1); heading.Children.Add(label);
        content.Children.Add(heading);
        if (detail.Length > 0)
        {
            var description = design.Text(detail, 11); description.Foreground = design.Brush("text-muted");
            if (model.Tabs.Selected == "artifacts") { description.MaxLines = 1; description.TextTrimming = TextTrimming.CharacterEllipsis; }
            content.Children.Add(description);
        }
        var button = new Button { Content = content, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        design.QuietButton(button); button.Padding = new Thickness(12); button.Background = design.Brush("bg-app");
        AutomationProperties.SetName(button, title); ToolTipService.SetToolTip(button, detail.Length > 0 ? title + "\n" + detail : title);
        return button;
    }
    private TextBlock Mute(string text) => new() { Text = text, Foreground = design.Brush("text-muted"), FontSize = 12, TextWrapping = TextWrapping.Wrap };
    private string Label(string id) => id switch
    {
        "artifacts" => design.T("right.artifacts"), "files" => design.T("right.file"), "hosts" => design.T("contexts.title"), "agents" => design.T("right.agents"),
        "runs" => model.SessionRuns.Active.Length is var n && n > 0 ? design.Localization.Format("right.runs_n", ("n", n.ToString())) : design.T("right.runs"), "memos" => design.T("right.memos"), "notebook" => design.L("笔记本", "Notebook"), "highlights" => design.L("摘录", "Highlights"), "provenance" => design.L("溯源", "Provenance"), "sidechat" => design.L("侧聊", "Side chat"), _ => id
    };
    private static string TabIcon(string id) => id switch
    {
        "runs" => "activity-orbit", "memos" => "notebook", "files" => "folder", "artifacts" => "doc", "agents" => "grid", "hosts" => "server",
        "notebook" => "book", "highlights" => "pin", "provenance" => "research-trail", _ => "chat"
    };
    public void Dispose()
    {
        if (disposed) return;
        transferTimer.Stop(); transferTimer.Tick -= PollFileTransfers;
        runsTimer.Stop(); runsTimer.Tick -= TickRuns; Loaded -= StartRunsPolling; Unloaded -= StopRunsPolling;
        foreach (var menu in openMenus.ToArray()) menu.Hide();
        fileDialog?.Hide();
        if (contextReviewDialog != null) contextReviewDialog.IsOpen = false;
        if (contextDialog != null) contextDialog.IsOpen = false;
        scroll.LayoutUpdated -= RestoreScroll;
        if (runReview != null) { runReview.Changed -= Render; runReview.Close(); }
        if (conversation != null) conversation.Changed -= RefreshTranscript;
        disposed = true; sourceSelectionActions?.Dispose(); artifactsPane.Dispose();
        lifetime.Cancel(); tabStrip.Dispose(); memosPane?.Dispose(); artifactDialog?.Dispose(); documentPreview?.Dispose(); model.Close(); lifetime.Dispose();
    }
}
