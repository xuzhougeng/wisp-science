using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Automation.Provider;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

/// <summary>The shared 15px checkbox used by the sidebar and deletion choices.</summary>
internal sealed class NativeSessionCheck : Button
{
    private readonly WispDesign design;
    private readonly Border mark;
    private readonly TextBlock label;
    public bool? Selected { get; private set; }
    public NativeSessionCheck(WispDesign design, string text, double size = 13, bool wrap = false, bool markOnly = false)
    {
        this.design = design; design.QuietButton(this); MinWidth = MinHeight = 0;
        Padding = new Thickness(10, 7, 10, 7); HorizontalAlignment = HorizontalAlignment.Stretch; HorizontalContentAlignment = HorizontalAlignment.Stretch;
        var row = new Grid { ColumnSpacing = 9 };
        row.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); row.ColumnDefinitions.Add(new());
        mark = new Border { Width = 15, Height = 15, BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(4), VerticalAlignment = wrap ? VerticalAlignment.Top : VerticalAlignment.Center, Margin = wrap ? new Thickness(0, 3, 0, 0) : new Thickness(0) };
        label = design.Text(text, size); label.VerticalAlignment = VerticalAlignment.Center;
        label.TextWrapping = wrap ? TextWrapping.WrapWholeWords : TextWrapping.NoWrap; label.TextTrimming = TextTrimming.CharacterEllipsis;
        Grid.SetColumn(label, 1); row.Children.Add(mark); row.Children.Add(label); Content = row; SetLabel(text); SetSelected(false);
        if (markOnly) { Width = Height = 15; Padding = new(0); row.ColumnSpacing = 0; label.Visibility = Visibility.Collapsed; }
    }
    public void SetLabel(string text) { label.Text = text; AutomationProperties.SetName(this, text); }
    public void SetSelected(bool? selected, bool highlight = false)
    {
        Selected = selected; var checkedMark = selected != false;
        mark.Background = design.Brush(checkedMark ? "clay" : "bg-elev"); mark.BorderBrush = design.Brush(checkedMark ? "clay" : "border-strong");
        mark.Child = checkedMark ? design.Icon(selected == null ? "minus" : "check", 11, design.ColorText("on-clay")) : null;
        Background = highlight && checkedMark ? design.Brush("clay-soft") : new Microsoft.UI.Xaml.Media.SolidColorBrush(Microsoft.UI.Colors.Transparent);
        BorderBrush = highlight && checkedMark ? design.Brush("clay") : new Microsoft.UI.Xaml.Media.SolidColorBrush(Microsoft.UI.Colors.Transparent); BorderThickness = highlight ? new Thickness(1) : new Thickness(0);
    }
    protected override AutomationPeer OnCreateAutomationPeer() => new CheckPeer(this);
    private sealed class CheckPeer(NativeSessionCheck owner) : ButtonAutomationPeer(owner), IToggleProvider
    {
        public ToggleState ToggleState => owner.Selected switch { true => ToggleState.On, false => ToggleState.Off, _ => ToggleState.Indeterminate };
        public void Toggle() { if (owner.IsEnabled) ((IInvokeProvider)this).Invoke(); }
        protected override AutomationControlType GetAutomationControlTypeCore() => AutomationControlType.CheckBox;
        protected override object GetPatternCore(PatternInterface patternInterface) => patternInterface == PatternInterface.Toggle ? this : base.GetPatternCore(patternInterface);
    }
}

internal sealed class NativeSessionSelection : UserControl
{
    private readonly WorkspaceSessionGroups model;
    private readonly WispDesign design;
    private readonly NativeSessionCheck all;
    private readonly Button move, delete, cancel;
    private readonly TextBlock error;
    private readonly List<(NativeSessionCheck Check, string[] Ids, bool Row)> checks = [];
    private readonly string[] ids;
    private bool moving;
    public NativeSessionSelection(WorkspaceSessionGroups model, IReadOnlyList<BrowserSession> sessions, WispDesign design,
        Action requestDelete, Action cancelSelection, Func<string?, Task> moveSelection, Action<FlyoutBase> register, string? active = null, bool rail = false)
    {
        this.model = model; this.design = design; ids = sessions.Select(s => s.Id).ToArray(); model.Selected.IntersectWith(ids);
        var root = new Grid { RowSpacing = 6 }; root.RowDefinitions.Add(new() { Height = GridLength.Auto }); root.RowDefinitions.Add(new());
        var controls = new StackPanel { Spacing = 4 }; var header = new Grid { Padding = new Thickness(10, 10, 10, 4) };
        header.ColumnDefinitions.Add(new()); header.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        all = new NativeSessionCheck(design, "", 12, wrap: true, markOnly: rail) { Padding = rail ? new Thickness(0) : new Thickness(2, 4, 6, 4) };
        all.Click += (_, _) => Toggle(ids); header.Children.Add(all);
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 3 };
        move = Tool(design.T("sidebar.move_selected"), "folder-move"); actions.Children.Add(move);
        delete = Tool(design.T("ctx.delete_session"), "trash"); delete.Click += (_, _) => requestDelete(); actions.Children.Add(delete);
        cancel = new Button { Content = design.Text(design.T("settings.cancel"), 11) }; design.QuietButton(cancel); cancel.Padding = new Thickness(6, 4, 6, 4); cancel.MinWidth = cancel.MinHeight = 0;
        cancel.Click += (_, _) => cancelSelection(); actions.Children.Add(cancel); Grid.SetColumn(actions, 1); header.Children.Add(actions); controls.Children.Add(header);
        if (rail)
        {
            all.HorizontalAlignment = HorizontalAlignment.Center; actions.Orientation = Orientation.Vertical; actions.HorizontalAlignment = HorizontalAlignment.Center;
            header.RowDefinitions.Add(new() { Height = GridLength.Auto }); header.RowDefinitions.Add(new() { Height = GridLength.Auto });
            Grid.SetColumn(actions, 0); Grid.SetRow(actions, 1); cancel.Content = design.Icon("close", 16);
            AutomationProperties.SetName(cancel, design.T("settings.cancel")); ToolTipService.SetToolTip(cancel, design.T("settings.cancel"));
        }
        error = design.Text("", 11); error.Foreground = design.Brush("err"); error.Margin = new Thickness(10, 0, 10, 0); controls.Children.Add(error); root.Children.Add(controls);
        var menu = new Flyout { Placement = FlyoutPlacementMode.BottomEdgeAlignedRight };
        var style = new Style(typeof(FlyoutPresenter));
        style.Setters.Add(new Setter(Control.PaddingProperty, new Thickness(8))); style.Setters.Add(new Setter(Control.BackgroundProperty, design.Brush("bg-elev")));
        style.Setters.Add(new Setter(Control.BorderBrushProperty, design.Brush("border-strong"))); style.Setters.Add(new Setter(Control.CornerRadiusProperty, new CornerRadius(10)));
        menu.FlyoutPresenterStyle = style;
        var destinations = new StackPanel { MinWidth = 166, MaxWidth = 220 };
        var title = design.Text(design.T("sidebar.move_selected"), 10.5); title.Foreground = design.Brush("text-faint"); title.Margin = new Thickness(6, 2, 6, 5); destinations.Children.Add(title);
        foreach (var folder in new[] { new ProjectFolder("", design.T("ctx.move_to_ungrouped")) }.Concat(model.Folders))
        {
            var text = string.IsNullOrEmpty(folder.Name) ? design.T("folder.untitled") : folder.Name;
            var label = design.Text(text, 13); label.TextWrapping = TextWrapping.NoWrap; label.TextTrimming = TextTrimming.CharacterEllipsis;
            var choice = new Button { Content = label, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch };
            design.QuietButton(choice); choice.Padding = new Thickness(8, 6, 8, 6); choice.MinWidth = choice.MinHeight = 0; AutomationProperties.SetName(choice, text);
            choice.Click += async (_, _) => { menu.Hide(); moving = true; Refresh(); try { await moveSelection(folder.Id.Length == 0 ? null : folder.Id); } finally { moving = false; Refresh(); } };
            destinations.Children.Add(choice);
        }
        menu.Content = new ScrollViewer { Content = destinations, MaxHeight = 320, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled };
        register(menu); move.Flyout = menu;
        var rows = new StackPanel { Spacing = 1 };
        var tree = model.Tree(sessions);
        NativeSessionCheck Check(BrowserSession session, bool inFolder)
        {
            var title = NativeSessionTree.Title(session, design.Localization);
            var check = new NativeSessionCheck(design, title, markOnly: rail);
            if (rail) { check.HorizontalAlignment = HorizontalAlignment.Center; check.Margin = new(0, 8, 0, 8); ToolTipService.SetToolTip(check, title); }
            else if (inFolder) check.Padding = new(18, 7, 10, 7);
            checks.Add((check, [session.Id], true)); check.Click += (_, _) => Toggle([session.Id]); return check;
        }
        foreach (var group in model.Sections(tree.Roots))
        {
            var body = new StackPanel { Spacing = 1 };
            var inFolder = group.FolderId != null;
            if (group.FolderId is { } folder)
            {
                // WebView's folder checkbox selects its own top-level rows;
                // nested branches/subagents remain individually selectable.
                var members = group.Sessions.Select(s => s.Id).ToArray();
                var check = new NativeSessionCheck(design, design.T("sidebar.select_all") + " · " + group.DisplayTitle(design.Localization), markOnly: true);
                checks.Add((check, members, false)); check.Click += (_, _) => Toggle(members);
                NativeSessionFolderHeader folderHeader = null!;
                void RefreshFolder() { var expanded = model.FolderExpanded(folder); body.Visibility = expanded ? Visibility.Visible : Visibility.Collapsed; folderHeader.SetExpanded(expanded); }
                folderHeader = new(group, design, () => { model.ToggleFolder(folder); RefreshFolder(); }, null, check, rail: rail);
                rows.Children.Add(folderHeader); body.Margin = new(rail ? 0 : 12, 0, 0, 2); RefreshFolder();
            }
            else if (!rail && group.LabelKey != null) rows.Children.Add(NativeSessionFolderHeader.SectionTitle(group, design, rows.Children.Count == 0));
            rows.Children.Add(body);
            foreach (var session in group.Sessions)
            {
                body.Children.Add(Check(session, inFolder));
                var children = tree.ChildrenOf(session.Id);
                if (children.Length > 0 && model.Nesting.Expanded(session.Id, children, active))
                {
                    var nested = new StackPanel { Spacing = 1 };
                    foreach (var child in children) nested.Children.Add(Check(child, inFolder));
                    body.Children.Add(new Border { Child = nested, BorderBrush = design.Brush("border"), BorderThickness = new(1, 0, 0, 0), Margin = new(rail ? 0 : inFolder ? 27 : 19, 0, 0, 0), Padding = new(rail ? 4 : 6, 0, 0, 0) });
                }
            }
        }
        var scroll = new ScrollViewer { Content = rows, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled }; Grid.SetRow(scroll, 1); root.Children.Add(scroll); Content = root; Refresh();
    }
    private Button Tool(string title, string icon)
    {
        var result = design.ToolButton(title, icon); design.QuietButton(result); result.Width = result.Height = 26; result.MinWidth = result.MinHeight = 0;
        result.Padding = new Thickness(0); result.Content = design.Icon(icon, 16, design.ColorText("text-faint")); return result;
    }
    private void Toggle(string[] members) { model.ToggleSelection(members); Refresh(); }
    private void Refresh()
    {
        var enabled = !moving && !model.Busy; var count = model.Selected.Count;
        all.SetLabel(count == 0 ? design.T("sidebar.select_all") : design.T("sidebar.selected_n").Replace("{n}", count.ToString()));
        all.SetSelected(count == 0 ? false : count == ids.Length ? true : null); all.IsEnabled = enabled;
        move.IsEnabled = delete.IsEnabled = enabled && count > 0; move.Opacity = delete.Opacity = move.IsEnabled ? 1 : .4; cancel.IsEnabled = enabled;
        if (model.MoveUnconfirmed) { move.IsEnabled = false; move.Opacity = .4; }
        foreach (var (check, members, row) in checks) { var selected = members.Count(model.Selected.Contains); check.SetSelected(selected == 0 ? false : selected == members.Length ? true : null, row); check.IsEnabled = enabled && members.Length > 0; }
        error.Text = model.Error ?? ""; error.Visibility = model.Error == null ? Visibility.Collapsed : Visibility.Visible;
    }
}
