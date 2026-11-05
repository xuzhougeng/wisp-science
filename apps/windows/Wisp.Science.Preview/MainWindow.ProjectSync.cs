using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Windows.ApplicationModel.DataTransfer;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class MainWindow
{
    private HomeProjectSync homeSync = new();
    private bool syncBackendConfigured;
    private string? homeSyncNotice, restoreSyncFocus;

    private FrameworkElement ProjectCardMetadata(ProjectSummary project)
    {
        var row = new NativeActionWrap { Gap = 10, RowGap = 4, VerticallyCentered = true, Margin = new Thickness(0, 2, 0, 0) };
        string Count(string key, long n) => design.Localization.Format(key, ("n", n.ToString()));
        void Activity(string key, long count, string color, string fill)
        {
            if (count <= 0) return;
            var label = Text(Count(key, count), 11, color); label.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
            var badge = new Border { Child = label, CornerRadius = new CornerRadius(99), Padding = new Thickness(8, 2, 8, 2), Background = design.Brush(fill) };
            if (key == "sess_status.needs_you_n")
            {
                var hint = design.T("sess_status.needs_you_hint");
                ToolTipService.SetToolTip(badge, hint); AutomationProperties.SetHelpText(label, hint);
            }
            row.Children.Add(badge);
        }
        Activity("sess_status.needs_you_n", project.NeedsYouCount, "info", "info-soft");
        Activity("sess_status.running_n", project.RunningCount, "clay", "clay-soft");
        row.Children.Add(Text(Count("projects.sessions_n", project.SessionCount), 13, "text-faint"));
        row.Children.Add(Text(Count("projects.artifacts_n", project.ArtifactCount), 13, "text-faint"));
        if (HomeProjectSync.Badge(project, DateTimeOffset.Now, design.Localization) is { } badge)
        {
            var state = Text(badge.Label, 11, badge.Color); state.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
            row.Children.Add(state);
        }
        return row;
    }

    private Border HomeSyncNotice(string message)
    {
        var label = Text(message, 13); AutomationProperties.SetLiveSetting(label, Microsoft.UI.Xaml.Automation.Peers.AutomationLiveSetting.Polite);
        var notice = design.Card(label); notice.CornerRadius = new CornerRadius(10); notice.MaxWidth = 440;
        notice.Padding = new Thickness(14, 10, 14, 10);
        var ok = design.Brush("ok").Color; var border = design.Brush("border").Color;
        var edge = NativeQueuePresentation.Mix((ok.A, ok.R, ok.G, ok.B), (border.A, border.R, border.G, border.B), .45);
        notice.BorderBrush = new Microsoft.UI.Xaml.Media.SolidColorBrush(Windows.UI.Color.FromArgb(edge.A, edge.R, edge.G, edge.B));
        notice.Margin = new Thickness(24); notice.HorizontalAlignment = HorizontalAlignment.Right; notice.VerticalAlignment = VerticalAlignment.Bottom;
        notice.IsHitTestVisible = false; return notice;
    }

    private bool OnHomeProject(string id) => !windowClosed && model.ActiveProjectId == null && examplePage == null
        && model.HomeProjects.Any(p => p.Id == id);
    private bool HomeUncovered => workspaceSheet == null && settingsPage == null && searchOverlay == null && !nativePickerOpen
        && projectImport?.IsModal != true && projectExport?.IsModal != true;

    private async Task CopyProjectCodeAsync(string id)
    {
        var scope = homeSync; var revision = model.NavigationRevision;
        bool Current() => scope == homeSync && revision == model.NavigationRevision && OnHomeProject(id) && HomeUncovered;
        try
        {
            var host = await ConnectHostAsync(); if (!Current()) return;
            if (host == null) throw new IOException(localError ?? design.L("无法连接桌面服务。", "Could not connect to the desktop service."));
            var value = await host.InvokeAsync("project_sync_code", new() { ["id"] = id }, id);
            if (!Current()) return;
            if (value is not System.Text.Json.Nodes.JsonValue text || !text.TryGetValue<string>(out var code) || string.IsNullOrWhiteSpace(code))
                throw new InvalidDataException(design.L("未收到设备码。", "No device code was returned."));
            var data = new DataPackage(); data.SetText(code);
            if (!Clipboard.SetContentWithOptions(data, new ClipboardContentOptions { IsAllowedInHistory = false, IsRoamable = false }))
                throw new IOException(design.L("无法复制设备码。", "Could not copy the device code."));
            localError = null; homeSyncNotice = design.T("projects.sync.code_copied"); Render();
        }
        catch (Exception ex) { if (Current()) { homeSyncNotice = null; localError = ex.Message; Render(); } }
    }

    private async Task RunHomeSyncAsync(string id, string? strategy = null, NativeHomeSyncConflict? dialog = null)
    {
        var scope = homeSync; var revision = model.NavigationRevision;
        bool Current() => scope == homeSync && revision == model.NavigationRevision && OnHomeProject(id);
        bool Ready() => dialog == null ? HomeUncovered : ReferenceEquals(workspaceSheet, dialog);
        if (!Current() || !Ready() || scope.Busy(id)) return;
        async Task<INativeSettingsClient> Connect()
        {
            var host = await ConnectHostAsync(); if (!Current() || !Ready()) throw new OperationCanceledException();
            return host ?? throw new IOException(localError ?? design.L("无法连接桌面服务。", "Could not connect to the desktop service."));
        }
        localError = null; homeSyncNotice = design.T("projects.sync.running");
        try
        {
            var running = scope.RunAsync(Connect, id, strategy, design.Localization);
            dialog?.SetPending(true); Render();
            var result = await running;
            if (!Current()) return;
            homeSyncNotice = result;
            if (result != null)
            {
                var closedConflict = dialog != null && ReferenceEquals(workspaceSheet, dialog);
                if (closedConflict) CloseSheet();
                await model.RefreshAsync();
                if (closedConflict && Current() && HomeUncovered) restoreSyncFocus = id;
            }
            else if (scope.InConflict(id) && Ready() && dialog == null)
            {
                NativeHomeSyncConflict? page = null;
                page = new(design, pick => _ = RunHomeSyncAsync(id, pick, page), () =>
                {
                    if (!ReferenceEquals(workspaceSheet, page) || scope.Busy(id)) return;
                    scope.Dismiss(id); restoreSyncFocus = id; CloseSheet(); Render();
                });
                MountSheet(page);
            }
        }
        catch (Exception ex)
        {
            if (!Current()) return;
            homeSyncNotice = null;
            if (dialog != null && ReferenceEquals(workspaceSheet, dialog)) dialog.ShowError(ex.Message); else localError = ex.Message;
        }
        finally { if (Current()) { dialog?.SetPending(false); Render(); } }
    }
}

internal sealed class NativeHomeSyncConflict : WorkspaceSheet
{
    private bool pending;
    private readonly NativeActionWrap choices = new() { Gap = 10, RightAligned = true };
    private readonly TextBlock error;
    public NativeHomeSyncConflict(WispDesign design, Action<string> resolve, Action close)
        : base(design, design.T("projects.sync.conflict_title"), close, modal: true, modalWidth: 480)
    {
        HideDismissButton();
        foreach (var key in new[] { "projects.sync.conflict_hint", "projects.sync.conflict_backup" })
        {
            var copy = Mute(design.T(key)); design.BindLineHeight(copy, 13, 1.55);
            var em = design.FontSize(13); copy.Margin = new Thickness(0, em, 0, em);
            Body.Children.Add(copy);
        }
        error = Mute(""); error.Foreground = design.Brush("err"); error.Visibility = Visibility.Collapsed;
        AutomationProperties.SetLiveSetting(error, Microsoft.UI.Xaml.Automation.Peers.AutomationLiveSetting.Polite); Body.Children.Add(error);
        Button Add(string key, Action action, bool primary = false)
        {
            var button = new Button { Content = design.T(key) }; design.ActionButton(button, primary);
            button.Padding = new Thickness(16, 9, 16, 9); button.CornerRadius = new CornerRadius(10);
            if (!primary) { button.Background = design.Brush("bg-input"); button.BorderBrush = design.Brush("border-strong"); }
            if (button.Content is TextBlock caption)
            {
                design.BindTypography(caption, 13.5);
                if (primary) caption.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
            }
            button.Click += (_, _) => { if (!pending) action(); }; choices.Children.Add(button); return button;
        }
        var cancel = Add("projects.cancel", close);
        Add("projects.sync.use_remote", () => resolve("remote")); Add("projects.sync.use_local", () => resolve("local"), true);
        Footer.Children.Add(choices); Footer.Visibility = Visibility.Visible; Loaded += (_, _) => cancel.Focus(FocusState.Programmatic);
    }
    public void SetPending(bool value) { pending = value; foreach (var button in choices.Children.OfType<Button>()) button.IsEnabled = !value; if (value) error.Visibility = Visibility.Collapsed; }
    public void ShowError(string message) { error.Text = message; error.Visibility = Visibility.Visible; }
    public override void HandleEscape() { if (!pending) base.HandleEscape(); }
}
