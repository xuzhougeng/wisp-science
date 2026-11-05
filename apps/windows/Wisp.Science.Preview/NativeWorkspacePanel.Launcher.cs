using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class NativeWorkspacePanel
{
    private readonly ContentControl launcherHost = new() { Visibility = Visibility.Collapsed,
        HorizontalContentAlignment = HorizontalAlignment.Stretch, VerticalContentAlignment = VerticalAlignment.Stretch };

    private void InitializeLauncher(Grid root)
    {
        Grid.SetRowSpan(launcherHost, 3); root.Children.Add(launcherHost);
    }

    private void RenderLauncher()
    {
        var root = new Grid { Background = design.Brush("bg-elev") };
        root.RowDefinitions.Add(new() { Height = GridLength.Auto }); root.RowDefinitions.Add(new());
        var header = new Grid { Padding = new(14, 8, 8, 8), BorderBrush = design.Brush("border"), BorderThickness = new(0, 0, 0, 1) };
        header.ColumnDefinitions.Add(new()); header.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var title = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, VerticalAlignment = VerticalAlignment.Center };
        title.Children.Add(design.Icon("compass", 16)); title.Children.Add(design.Text(design.T("right.launcher.start"), 13)); header.Children.Add(title);
        var dismiss = design.ToolButton(design.T("right.close"), "close"); dismiss.Click += (_, _) => close();
        Grid.SetColumn(dismiss, 1); header.Children.Add(dismiss); root.Children.Add(header);
        var content = new StackPanel { Spacing = 10, Padding = new(18, 20, 18, 18) };
        var intro = new StackPanel { Spacing = 8, Margin = new(0, 0, 0, 12) };
        var compass = design.Icon("compass", 28); compass.HorizontalAlignment = HorizontalAlignment.Left; intro.Children.Add(compass);
        var heading = design.Text(design.T("right.launcher.title"), 20); heading.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        intro.Children.Add(heading); intro.Children.Add(design.Text(design.T("right.launcher.subtitle"), 12, "text-muted")); content.Children.Add(intro);
        foreach (var tool in new[] {
            ("files", "folder", "right.launcher.files", "right.launcher.files_hint"),
            ("agents", "swarm", "right.agents", "right.launcher.agents_hint"),
            ("artifacts", "layers", "right.artifacts", "right.launcher.artifacts_hint"),
            ("hosts", "server", "contexts.title", "right.launcher.hosts_hint") })
            if (model.Tabs.Available.Contains(tool.Item1)) content.Children.Add(LauncherTool(tool));
        var more = new StackPanel { Spacing = 10 };
        foreach (var tool in new[] {
            ("runs", "activity-orbit", "right.runs", "right.launcher.runs_hint"),
            ("notebook", "notebook", "right.notebook", "right.launcher.notebook_hint"),
            ("highlights", "star", "right.highlights", "right.launcher.highlights_hint"),
            ("provenance", "branch", "right.provenance", "right.launcher.provenance_hint"),
            ("sidechat", "chat", "sidechat.title", "right.launcher.sidechat_hint"),
            ("memos", "memory", "right.memos", "right.launcher.memos_hint") })
            if (model.Tabs.Available.Contains(tool.Item1)) more.Children.Add(LauncherTool(tool));
        if (more.Children.Count > 0)
        {
            var copy = new StackPanel { Spacing = 4 };
            copy.Children.Add(design.Text(design.T("right.launcher.more"), 13));
            copy.Children.Add(design.Text(design.T("right.launcher.more_hint"), 12, "text-muted"));
            var expander = new Expander { Header = copy, Content = more, HorizontalAlignment = HorizontalAlignment.Stretch,
                HorizontalContentAlignment = HorizontalAlignment.Stretch };
            AutomationProperties.SetName(expander, design.T("right.launcher.more")); content.Children.Add(expander);
        }
        var scroller = new ScrollViewer { Content = content, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled };
        Grid.SetRow(scroller, 1); root.Children.Add(scroller); launcherHost.Content = root;
        AutomationProperties.SetAutomationId(root, "right-pane-launcher"); design.ApplyTypography(root);
    }

    private Button LauncherTool((string Id, string Icon, string Title, string Hint) tool)
    {
        var row = new Grid { ColumnSpacing = 12 };
        row.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); row.ColumnDefinitions.Add(new()); row.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        row.Children.Add(design.Icon(tool.Icon, 20));
        var copy = new StackPanel { Spacing = 4 };
        var title = design.Text(design.T(tool.Title), 13); title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        copy.Children.Add(title); copy.Children.Add(design.Text(design.T(tool.Hint), 12, "text-muted")); Grid.SetColumn(copy, 1); row.Children.Add(copy);
        var arrow = design.Icon("chevron-right", 14); Grid.SetColumn(arrow, 2); row.Children.Add(arrow);
        var button = new Button { Content = row, Padding = new(12), CornerRadius = new(10), Background = design.Brush("bg-elev"),
            BorderBrush = design.Brush("border"), HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        AutomationProperties.SetName(button, design.T(tool.Title)); AutomationProperties.SetAutomationId(button, "right-launcher-" + tool.Id);
        button.Click += async (_, _) => await ShowTabAsync(tool.Id); return button;
    }
}
