using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed class NativeShelvedSessionsPage : WorkspaceSheet
{
    private readonly WorkspaceShelvedSessions model;
    private readonly StackPanel rows = new() { Spacing = 0, MinHeight = 60, Padding = new Thickness(4) };
    private readonly TextBlock error;
    private readonly Button more;
    private readonly Func<ShelvedSession, Task> open;
    private readonly Func<Task> restored;
    private readonly Action<FlyoutBase> register;
    private readonly List<NativeSessionMenu> contextMenus = [];
    private bool disposed;

    public NativeShelvedSessionsPage(WorkspaceShelvedSessions model, WispDesign design, Func<ShelvedSession, Task> open,
        Func<Task> restored, Action close, Action<FlyoutBase> register) : base(design, design.T("session.shelved"), close, modal: true, modalWidth: 680)
    {
        this.model = model; this.open = open; this.restored = restored; this.register = register;
        Body.Padding = new Thickness(22, 0, 22, 22);
        Body.Children.Add(Label(design.T("session.shelved_hint"), 13, "text-muted"));
        var search = new TextBox { PlaceholderText = design.T("session.shelved_search"), MinHeight = 38,
            Padding = new Thickness(11, 9, 11, 9), CornerRadius = new CornerRadius(10) };
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(search, design.T("session.shelved_search"));
        design.BindTypography(search, size: 14);
        PreferenceFieldAppearance.Apply(search, design);
        var textMenu = new TextCommandBarFlyout(); search.ContextFlyout = textMenu; register(textMenu);
        if (Content is Grid overlay) overlay.PointerPressed += (_, e) => { if (ReferenceEquals(e.OriginalSource, overlay)) HandleEscape(); };
        search.TextChanged += async (_, _) => await model.SearchAsync(search.Text);
        Body.Children.Add(search);
        error = Label("", 13, "clay-strong"); error.Visibility = Visibility.Collapsed; Body.Children.Add(error);
        Body.Children.Add(rows);
        more = new Button { HorizontalAlignment = HorizontalAlignment.Center };
        design.QuietButton(more); more.Click += async (_, _) => await model.MoreAsync(); Body.Children.Add(more);
        model.Changed += Refresh;
        Loaded += async (_, _) => { if (!disposed) await model.SearchAsync(search.Text); };
        Refresh();
    }
    private void Refresh()
    {
        if (disposed) return;
        foreach (var menu in contextMenus) menu.Hide(); contextMenus.Clear();
        rows.Children.Clear(); error.Visibility = model.Error == null ? Visibility.Collapsed : Visibility.Visible;
        error.Text = model.Error == null ? "" : (model.RestoreUnconfirmed
            ? Design.L("恢复结果未能确认，请关闭后重新打开检查；不会自动重试。\n", "Restore could not be confirmed. Reopen to check; it will not be retried automatically.\n") : "") + model.Error;
        foreach (var item in model.Items)
        {
            var row = new Grid { ColumnSpacing = 12, Padding = new Thickness(0, 8, 0, 8), BorderThickness = new Thickness(0, 0, 0, 1), BorderBrush = Design.Brush("border") };
            row.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); row.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            row.RowDefinitions.Add(new() { Height = GridLength.Auto }); row.RowDefinitions.Add(new() { Height = GridLength.Auto });
            var label = new Grid { ColumnSpacing = 8 };
            label.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); label.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
            var icon = Design.Icon("chat", 16, Design.ColorText("text-muted")); icon.VerticalAlignment = VerticalAlignment.Top; icon.Margin = new Thickness(0, 3, 0, 0); label.Children.Add(icon);
            var title = Design.Text(item.Title, 14); title.TextWrapping = TextWrapping.WrapWholeWords; Grid.SetColumn(title, 1); label.Children.Add(title);
            var button = new Button { Content = label, HorizontalContentAlignment = HorizontalAlignment.Stretch, HorizontalAlignment = HorizontalAlignment.Stretch, Padding = new Thickness(8), MinWidth = 0 };
            Design.QuietButton(button); button.Padding = new Thickness(8); button.Click += async (_, _) => await open(item);
            Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(button, item.Title); row.Children.Add(button);
            var context = new NativeSessionMenu(Design, NativeSessionMenuEntries.Shelved(model.RestoringId == null && !model.RestoreUnconfirmed), entry =>
            {
                if (disposed) return;
                if (entry.Id == "open") _ = open(item); else _ = Restore(item.Id);
            }, register);
            contextMenus.Add(context); button.ContextFlyout = context; row.ContextFlyout = context;
            var restoreLabel = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 };
            restoreLabel.Children.Add(Design.Icon("undo", 16, Design.ColorText("clay-strong"))); restoreLabel.Children.Add(Label(Design.T("session.restore"), 12, "clay-strong"));
            var restore = new Button { Content = restoreLabel, MinHeight = 34, MinWidth = 0, Padding = new Thickness(6, 6, 12, 6), CornerRadius = new CornerRadius(6), VerticalAlignment = VerticalAlignment.Center,
                Background = Design.Brush("clay-soft"), BorderBrush = Design.Brush("border-strong"), IsEnabled = model.RestoringId == null && !model.RestoreUnconfirmed };
            Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(restore, Design.T("session.restore") + " " + item.Title);
            restore.Opacity = restore.IsEnabled ? 1 : .45;
            restore.Click += async (_, _) => await Restore(item.Id);
            Grid.SetColumn(restore, 1); row.Children.Add(restore); rows.Children.Add(row);
            row.SizeChanged += (_, _) =>
            {
                var narrow = row.ActualWidth <= 476;
                Grid.SetColumnSpan(button, narrow ? 2 : 1); Grid.SetColumn(restore, narrow ? 0 : 1); Grid.SetRow(restore, narrow ? 1 : 0);
                Grid.SetColumnSpan(restore, narrow ? 2 : 1); restore.HorizontalAlignment = narrow ? HorizontalAlignment.Right : HorizontalAlignment.Stretch;
                row.ColumnDefinitions[1].Width = narrow ? new GridLength(0) : GridLength.Auto;
            };
        }
        if (model.Items.Length == 0 && !model.Loading && model.Error == null) rows.Children.Add(Label(Design.T("session.shelved_empty"), 14, "text-muted"));
        more.Visibility = model.Loading || model.NextCursor != null ? Visibility.Visible : Visibility.Collapsed;
        more.IsEnabled = !model.Loading; more.Content = Label(Design.T(model.Loading ? "loading" : "sidebar.load_older"), 12, "text-muted");
        Design.ApplyTypography(this);
    }
    private TextBlock Label(string text, double size, string color) { var label = Design.Text(text, size); label.Foreground = Design.Brush(color); return label; }
    private async Task Restore(string id) { if (await model.RestoreAsync(id) && !disposed) await restored(); }
    public override void Dispose() { disposed = true; foreach (var menu in contextMenus) menu.Hide(); contextMenus.Clear(); model.Changed -= Refresh; model.Dispose(); base.Dispose(); }
}
