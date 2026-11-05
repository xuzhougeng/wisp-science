using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

/// <summary>Shared production sidebar family renderer for the workbench and
/// database-free visual review. Disclosure changes preserve mounted row focus.</summary>
internal sealed class NativeSessionTreeView : StackPanel
{
    private readonly Dictionary<string, NativeSessionTreeRow> rows = [];
    private readonly List<Action> refreshFamilies = [];
    public NativeSessionTreeView(BrowserSession[] sessions, WorkspaceSessionGroups? groups, WispDesign design,
        string? active, Action<BrowserSession> open, Func<BrowserSession, FlyoutBase?> menu,
        Action<FrameworkElement, string, string> drag, Action<string> renameFolder,
        Func<string, FlyoutBase?>? folderMenu = null, Action<Border, string?>? drop = null,
        Action<NativeExplorationSummary>? openExploration = null, Func<NativeExplorationSummary, FlyoutBase?>? explorationMenu = null, bool rail = false)
    {
        Spacing = 1;
        var tree = groups?.Tree(sessions) ?? NativeSessionTree.Build(sessions);
        var nesting = groups?.Nesting ?? new NativeSessionNesting();
        var sections = groups?.Sections(tree.Roots) ?? [new SessionSection("", null, tree.Roots)];
        NativeSessionTreeRow Row(BrowserSession session, bool nested, bool inFolder)
        {
            var row = new NativeSessionTreeRow(session, design, active == session.Id, nested, () => open(session), menu(session), inFolder, rail);
            rows.Add(session.Id, row); drag(row.OpenButton, "session", session.Id); return row;
        }
        var ungrouped = new StackPanel { Spacing = 1 };
        foreach (var section in sections)
        {
            var body = new StackPanel { Spacing = 1 };
            var inFolder = section.FolderId != null;
            Panel destination = groups?.Group == "folder" && !inFolder && section.LabelKey != "sidebar.pinned" ? ungrouped : this;
            if (groups != null && section.FolderId is { } folder)
            {
                NativeSessionFolderHeader header = null!;
                void RefreshFolder() { var expanded = groups.FolderExpanded(folder); body.Visibility = expanded ? Visibility.Visible : Visibility.Collapsed; header.SetExpanded(expanded); }
                header = new(section, design, () => { groups.ToggleFolder(folder); RefreshFolder(); }, () => renameFolder(folder), menu: folderMenu?.Invoke(folder), rail: rail);
                drag(header.ToggleButton, "folder", folder); body.Margin = new(rail ? 0 : 12, 0, 0, 2); RefreshFolder();
                var content = new StackPanel { Spacing = 1 }; content.Children.Add(header); content.Children.Add(body);
                var wrapper = new Border { Child = content, CornerRadius = new(6), Margin = new(0, 0, 0, 2) }; drop?.Invoke(wrapper, folder); Children.Add(wrapper);
            }
            else
            {
                if (!rail && section.LabelKey != null) destination.Children.Add(NativeSessionFolderHeader.SectionTitle(section, design, Children.Count == 0 && destination.Children.Count == 0));
                destination.Children.Add(body);
            }
            foreach (var session in section.Sessions)
            {
                var parent = Row(session, false, inFolder); body.Children.Add(parent);
                var children = tree.ChildrenOf(session.Id);
                var explorations = groups?.Explorations.Where(row => row.SourceFrameId == session.Id).ToArray() ?? [];
                if (children.Length + explorations.Length == 0) continue;
                var nested = new StackPanel { Spacing = 1 };
                foreach (var exploration in explorations)
                    nested.Children.Add(new NativeExplorationRow(exploration, design, active == exploration.Exploration.FrameId,
                        () => openExploration?.Invoke(exploration), explorationMenu?.Invoke(exploration), rail));
                foreach (var child in children) nested.Children.Add(Row(child, true, inFolder));
                var guide = new Border { Child = nested, BorderBrush = design.Brush("border"), BorderThickness = new(1, 0, 0, 0),
                    Margin = new(rail ? 0 : inFolder ? 27 : 19, 0, 0, 0), Padding = new(rail ? 4 : 6, 0, 0, 0) };
                body.Children.Add(guide);
                bool Expanded() => nesting.Expanded(session.Id, children.Select(c => c with { Status = rows[c.Id].Status })
                    .Concat(explorations.Select(row => row.Session(session.ProjectId))), active);
                void Refresh() { var expanded = Expanded(); guide.Visibility = expanded ? Visibility.Visible : Visibility.Collapsed; parent.Disclosure(children.Length + explorations.Length, expanded); }
                parent.Toggle += () => { nesting.SetExpanded(session.Id, !Expanded()); Refresh(); };
                refreshFamilies.Add(Refresh); Refresh();
            }
        }
        if (groups?.Group == "folder" && (ungrouped.Children.Count > 0 || groups.Folders.Length > 0))
        {
            var wrapper = new Border { Child = ungrouped, CornerRadius = new(6), MinHeight = 24 }; drop?.Invoke(wrapper, null); Children.Add(wrapper);
        }
    }
    public void UpdateActivity(string sessionId, string status)
    {
        if (!rows.TryGetValue(sessionId, out var row)) return;
        row.UpdateActivity(status); foreach (var refresh in refreshFamilies) refresh();
    }
}

internal sealed class NativeSessionTreeRow : Grid
{
    private readonly BrowserSession session; private readonly WispDesign design; private readonly bool inFolder;
    private readonly Grid statusIcon = new() { Width = 16, Height = 16 };
    private readonly Border badge; private readonly TextBlock badgeText, count;
    private readonly TextBlock? initial;
    private readonly Button? more; private Button? toggle; private bool hovered, focused, expanded; private int childCount;
    public Button OpenButton { get; }
    public string Status { get; private set; }
    public event Action? Toggle;
    public NativeSessionTreeRow(BrowserSession session, WispDesign design, bool active, bool nested, Action open, FlyoutBase? menu, bool inFolder = false, bool rail = false)
    {
        this.session = session; this.design = design; this.inFolder = inFolder; Status = session.Status;
        var title = NativeSessionTree.Title(session, design.Localization);
        var content = new Grid { ColumnSpacing = 9 };
        content.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); content.ColumnDefinitions.Add(new()); content.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        content.Children.Add(statusIcon);
        var label = design.Text(title, 13); label.TextWrapping = TextWrapping.NoWrap; label.TextTrimming = TextTrimming.CharacterEllipsis;
        label.VerticalAlignment = VerticalAlignment.Center; label.Foreground = design.Brush(active ? "text" : "text-muted"); Grid.SetColumn(label, 1); content.Children.Add(label);
        badgeText = design.Text("", 10); badgeText.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; badgeText.TextWrapping = TextWrapping.NoWrap;
        badge = new Border { Child = badgeText, Padding = new(5, 1, 5, 1), CornerRadius = new(4), VerticalAlignment = VerticalAlignment.Center }; Grid.SetColumn(badge, 2); content.Children.Add(badge);
        OpenButton = new Button { Content = content, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        design.QuietButton(OpenButton); OpenButton.MinWidth = OpenButton.MinHeight = 0; OpenButton.Padding = new(inFolder ? 18 : nested ? 9 : 10, 6, 34, 6); OpenButton.BorderThickness = new(1); OpenButton.BorderBrush = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        if (active) OpenButton.Background = design.Brush(nested ? "clay-soft" : "bg-elev");
        if (rail)
        {
            initial = design.Text(NativeSessionTree.RailInitial(session, design.Localization), 12);
            initial.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; initial.Foreground = design.Brush(active ? "clay-strong" : "text-muted");
            initial.VerticalAlignment = VerticalAlignment.Center; initial.HorizontalAlignment = HorizontalAlignment.Center;
            content.Children.Remove(statusIcon);
            var glyph = new Grid(); glyph.Children.Add(initial); glyph.Children.Add(statusIcon);
            OpenButton.Content = glyph; OpenButton.MinHeight = 32; OpenButton.Padding = new(4);
            OpenButton.HorizontalContentAlignment = HorizontalAlignment.Center;
        }
        OpenButton.Click += (_, _) => open(); AutomationProperties.SetAutomationId(OpenButton, "session-" + session.Id);
        if (NativeSessionTree.Kind(session) is { } kind) AutomationProperties.SetHelpText(OpenButton, design.T(kind == "bot" ? "subagent.group" : "branch.group"));
        Children.Add(OpenButton);
        count = design.Text("", 11); count.Foreground = design.Brush("text-faint"); count.HorizontalAlignment = HorizontalAlignment.Right; count.VerticalAlignment = VerticalAlignment.Center; count.Margin = new(0, 0, 12, 0); count.IsHitTestVisible = false; Children.Add(count);
        if (menu != null)
        {
            more = design.ToolButton(design.T("session.actions"), "more"); design.QuietButton(more); more.Content = design.Icon("more", 14, design.ColorText("text-faint"));
            more.Width = more.Height = 25; more.MinWidth = more.MinHeight = 0; more.Padding = new(0); more.Margin = new(0, 0, 5, 0);
            more.HorizontalAlignment = HorizontalAlignment.Right; more.VerticalAlignment = VerticalAlignment.Center; more.Background = design.Brush("bg-elev");
            more.Flyout = menu; OpenButton.ContextFlyout = menu; ContextFlyout = menu; Unloaded += (_, _) => menu.Hide(); Children.Add(more);
            if (rail) more.Visibility = Visibility.Collapsed;
        }
        PointerEntered += (_, _) => { hovered = true; Chrome(); }; PointerExited += (_, _) => { hovered = false; Chrome(); };
        GotFocus += (_, _) => { focused = true; Chrome(); }; LostFocus += (_, _) => { focused = false; Chrome(); };
        UpdateActivity(Status);
    }
    public void UpdateActivity(string status)
    {
        Status = status; statusIcon.Children.Clear(); var live = status is "running" or "needs_you";
        var kind = NativeSessionTree.Kind(session);
        if (status == "running") statusIcon.Children.Add(NativeLoadingVisuals.Spinner(design, 16));
        else if (status == "needs_you") statusIcon.Children.Add(design.Icon("circle-alert", 16, design.ColorText("warn")));
        else if (initial == null && kind != null) statusIcon.Children.Add(design.Icon(kind, 15, design.ColorText("clay")));
        if (initial != null) { initial.Visibility = live ? Visibility.Collapsed : Visibility.Visible; statusIcon.Visibility = live ? Visibility.Visible : Visibility.Collapsed; }
        badge.Visibility = live ? Visibility.Visible : Visibility.Collapsed; badgeText.Text = live ? design.T("sess_status." + status) : "";
        badgeText.Foreground = design.Brush(status == "needs_you" ? "warn" : "clay");
        badge.Background = status == "needs_you"
            ? new SolidColorBrush(design.Brush("warn").Color) { Opacity = .1 }
            : design.Brush("clay-soft");
        var title = NativeSessionTree.Title(session, design.Localization) + (live ? " · " + badgeText.Text : "");
        AutomationProperties.SetName(OpenButton, title); ToolTipService.SetToolTip(OpenButton, title); Chrome();
    }
    public void Disclosure(int children, bool isExpanded)
    {
        childCount = children; expanded = isExpanded;
        if (initial != null) return;
        if (toggle == null)
        {
            toggle = new Button { Width = 18, Height = 18, HorizontalAlignment = HorizontalAlignment.Left, VerticalAlignment = VerticalAlignment.Center, Margin = new(inFolder ? 18 : 10, 0, 0, 0) };
            design.QuietButton(toggle); toggle.MinWidth = toggle.MinHeight = 0; toggle.Padding = new(0); toggle.Click += (_, _) => Toggle?.Invoke(); Children.Add(toggle);
        }
        toggle.Content = design.Icon(expanded ? "chevron-down" : "chevron-right", 13, design.ColorText("text-faint"));
        var label = design.T(expanded ? "sidebar.nest_collapse" : "sidebar.nest_expand"); AutomationProperties.SetName(toggle, label); ToolTipService.SetToolTip(toggle, label);
        AutomationProperties.SetItemStatus(toggle, expanded ? "expanded" : "collapsed"); count.Text = children.ToString(); Chrome();
    }
    private void Chrome()
    {
        var reveal = hovered || focused;
        if (more != null) { more.Opacity = reveal ? 1 : 0; more.IsHitTestVisible = reveal; }
        count.Visibility = initial == null && childCount > 0 && !expanded && !reveal ? Visibility.Visible : Visibility.Collapsed;
        if (toggle != null) { toggle.Opacity = reveal || Status is not ("running" or "needs_you") ? 1 : 0; toggle.Background = reveal ? design.Brush("bg-elev") : new SolidColorBrush(Microsoft.UI.Colors.Transparent); }
    }
}
