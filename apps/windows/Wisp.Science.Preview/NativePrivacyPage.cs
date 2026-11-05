using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed class NativePrivacyPage : WorkspaceSheet
{
    private readonly Action close;
    private readonly Func<PrivacyMode, Task<PrivacyMode>> save;
    private readonly Action<PrivacyMode?> accept;
    private readonly StackPanel list = new() { Spacing = 4 };
    private readonly Dictionary<string, CheckBox> checks = new(StringComparer.Ordinal);
    private readonly HashSet<string> selected = new(StringComparer.Ordinal);
    private readonly Button cancel, restore, hide;
    private readonly TextBlock status;
    private CheckBox? all;
    private PrivacyMode? original;
    private bool busy, syncing, disposed;

    internal NativePrivacyPage(WispDesign d, Action close, Func<PrivacyMode,Task<PrivacyMode>> save, Action<PrivacyMode?> accept)
        : base(d, d.T("privacy.title"), close, modal: true, modalWidth: 560, lightDismiss: true)
    {
        this.close = close; this.save = save; this.accept = accept;
        AutomationProperties.SetAutomationId(this, "privacy-mode");
        var head = new Grid { ColumnSpacing = 12 };
        head.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        head.ColumnDefinitions.Add(new() { Width = new GridLength(1,GridUnitType.Star) });
        head.Children.Add(new Border { Width = 36, Height = 36, VerticalAlignment = VerticalAlignment.Top, CornerRadius = new(8),
            Background = d.Brush("clay-soft"), Child = d.Icon("eye-off", 18, d.ColorText("clay-strong")) });
        var labels = new StackPanel { Spacing = 5 };
        var title = d.Text(d.T("privacy.title"), 17); title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        labels.Children.Add(title); var hint = d.Text(d.T("privacy.hint"), 13, "text-muted");
        hint.TextWrapping = TextWrapping.Wrap; labels.Children.Add(hint); Grid.SetColumn(labels,1); head.Children.Add(labels);
        UseScrollingHeader(head);
        Body.Padding = new Thickness(22,22,22,0);
        var listScroll = new ScrollViewer { Content = list, MaxHeight = 360, VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
            HorizontalScrollMode = ScrollMode.Disabled, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled };
        SizeChanged += (_,e) => listScroll.MaxHeight = Math.Min(360, e.NewSize.Height * .48);
        Body.Children.Add(listScroll);
        Footer.Visibility = Visibility.Visible; Footer.Spacing = 10;
        status = d.Text(d.L("正在读取…","Loading…"),12,"text-muted"); status.TextWrapping = TextWrapping.Wrap;
        status.VerticalAlignment = VerticalAlignment.Center;
        var foot = new NativeActionWrap { SpaceBetween = true, VerticallyCentered = true, Gap = 12 };
        foot.Children.Add(status); Footer.Children.Add(foot);
        var actions = new NativeActionWrap(); foot.Children.Add(actions);
        Button Add(string key, Action act, bool primary = false)
        {
            var b = new Button { Content = d.T(key) }; d.ActionButton(b, primary);
            if (primary)
            {
                b.Resources["ButtonBackgroundDisabled"] = b.Resources["ButtonBorderBrushDisabled"] = d.Brush("clay");
                b.Resources["ButtonForegroundDisabled"] = new Microsoft.UI.Xaml.Media.SolidColorBrush(Microsoft.UI.Colors.White);
            }
            b.Click += (_,_) => act(); actions.Children.Add(b); return b;
        }
        cancel = Add("projects.cancel", HandleEscape); restore = Add("privacy.restore", () => _ = CommitAsync(false));
        hide = Add("privacy.hide", () => _ = CommitAsync(true), true);
        restore.IsEnabled = false;
        SetHideEnabled(false);
        Loaded += (_,_) => cancel.Focus(FocusState.Programmatic);
    }
    internal async Task LoadAsync(Func<Task<ProjectListSnapshot>> read)
    {
        try
        {
            var snapshot = await read(); if (disposed) return;
            original = snapshot.Privacy is { ProjectIds: not null } mode ? mode : throw new InvalidDataException();
            accept(original);
            selected.UnionWith(original.ProjectIds);
            if (snapshot.Projects.Count > 0) all = Row(Design.T("privacy.select_all"), null, null);
            foreach (var p in snapshot.Projects) checks[p.Id] = Row(p.Name, p.WorkspaceDirectory, p.Id);
            status.Text = Design.T(original.Active ? "privacy.active" : "privacy.inactive"); Sync();
        }
        catch { if (!disposed) status.Text = Design.L("未能读取隐私设置，请关闭后重新打开。","Could not read privacy settings. Close and reopen to try again."); }
    }
    private CheckBox Row(string title, string? path, string? id)
    {
        var labels = new StackPanel { Spacing = 2, VerticalAlignment = VerticalAlignment.Center };
        var name = Design.Text(title,13); name.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; labels.Children.Add(name);
        if (!string.IsNullOrWhiteSpace(path))
        {
            var detail = Design.Text(path,11,"text-faint"); detail.FontFamily = Design.CodeFont; detail.TextTrimming = TextTrimming.CharacterEllipsis;
            ToolTipService.SetToolTip(detail,path); labels.Children.Add(detail);
        }
        var check = new CheckBox { Content = labels, HorizontalAlignment = HorizontalAlignment.Stretch,
            HorizontalContentAlignment = HorizontalAlignment.Stretch, MinWidth = 0, Padding = new Thickness(10,5,10,5) };
        AutomationProperties.SetName(check,title); AutomationProperties.SetAutomationId(check,id == null ? "privacy-select-all" : "privacy-project-" + id);
        var row = new Border { Child = check, Padding = new Thickness(9,2,9,2), CornerRadius = new(7), BorderThickness = new(1) }; list.Children.Add(row);
        check.Click += (_,_) => {
            if (syncing || busy) return;
            if (id == null) { selected.Clear(); if (check.IsChecked == true) selected.UnionWith(checks.Keys); }
            else if (check.IsChecked == true) selected.Add(id); else selected.Remove(id);
            Sync();
        };
        return check;
    }
    private void Sync()
    {
        syncing = true;
        void Mark(CheckBox check, bool value) {
            check.IsChecked = value; check.IsEnabled = !busy;
            var row = (Border)check.Parent; row.Background = Design.Brush(value ? "clay-soft" : "bg-sunken");
            row.BorderBrush = Design.Brush(value ? "clay" : "border");
        }
        if (all != null) Mark(all, checks.Count > 0 && checks.Keys.All(selected.Contains));
        foreach (var (id,check) in checks) Mark(check, selected.Contains(id));
        cancel.IsEnabled = !busy; restore.IsEnabled = !busy && original?.Active == true;
        SetHideEnabled(!busy && original != null && selected.Count > 0); syncing = false;
    }
    private void SetHideEnabled(bool enabled)
    {
        hide.IsEnabled = enabled;
        hide.Opacity = enabled ? 1 : .5;
    }
    private async Task CommitAsync(bool hideSelected)
    {
        if (busy || original == null || disposed) return;
        busy = true; Sync();
        try
        {
            var saved = await save(new(hideSelected, hideSelected ? selected.Order(StringComparer.Ordinal).ToArray() : original.ProjectIds));
            if (disposed) return;
            accept(saved); close();
        }
        catch
        {
            if (disposed) return;
            // A timed-out write may have committed. Conceal home rows until a
            // fresh snapshot is read, and never retry a privacy write automatically.
            accept(null); original = null; busy = false; list.IsHitTestVisible = false; Sync();
            status.Text = Design.L("未能确认保存结果。请关闭后重新打开以核对。","Could not confirm the change. Close and reopen to check the saved selection.");
        }
    }
    public override void HandleEscape() { if (!busy) close(); }
    public override void Dispose() { disposed = true; base.Dispose(); }
}
