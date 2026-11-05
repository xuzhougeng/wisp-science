using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed class NativeGeneratedDirectory : StackPanel
{
    private sealed class DirectoryNode
    {
        public SortedDictionary<string, DirectoryNode> Directories { get; } = new(StringComparer.Ordinal);
        public List<NativeGeneratedOutput> Files { get; } = [];
        public int Count => Files.Count + Directories.Values.Sum(node => node.Count);
    }
    public NativeGeneratedDirectory(NativeGeneratedOutput[] outputs, string projectRoot, WispDesign design,
        Action<NativeGeneratedOutput> open, IDictionary<string, bool> choices, string sourceKey)
    {
        var tree = new DirectoryNode(); Spacing = 6; HorizontalAlignment = HorizontalAlignment.Stretch;
        foreach (var output in outputs)
        {
            var node = tree;
            foreach (var part in NativeGeneratedArtifacts.Directory(output, projectRoot).Split('/', StringSplitOptions.RemoveEmptyEntries))
            {
                if (!node.Directories.TryGetValue(part, out var child)) node.Directories[part] = child = new();
                node = child;
            }
            node.Files.Add(output);
        }
        var label = design.Localization.Format("artifact.generated_count", ("n", tree.Count.ToString())).ToUpperInvariant();
        var border = new Border { Child = new ScrollViewer { Content = Contents(tree, sourceKey), MaxHeight = 360, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch },
            Padding = new(6), CornerRadius = new(8), BorderThickness = new(1), BorderBrush = design.Brush("border") };
        var directory = (FrameworkElement)Toggle(label, border, sourceKey, true);
        directory.HorizontalAlignment = HorizontalAlignment.Left;
        Children.Add(directory);
        SizeChanged += (_, e) => directory.Width = Math.Min(680, Math.Max(0, e.NewSize.Width));

        UIElement Contents(DirectoryNode node, string path)
        {
            var content = new StackPanel { Spacing = 3 };
            foreach (var (name, child) in node.Directories)
            {
                var childPath = path + "/" + name;
                var children = new Border { Child = Contents(child, childPath), Margin = new(12, 0, 0, 0), Padding = new(6, 0, 0, 0), BorderThickness = new(1, 0, 0, 0), BorderBrush = design.Brush("border") };
                content.Children.Add(Toggle(name + "  " + child.Count, children, childPath, false));
            }
            foreach (var output in node.Files)
            {
                var row = new Grid { ColumnSpacing = 8 }; row.ColumnDefinitions.Add(new() { Width = new GridLength(24) }); row.ColumnDefinitions.Add(new()); row.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
                row.Children.Add(new Border { Width = 24, Height = 24, CornerRadius = new(4), Background = design.Brush("clay-soft"), Child = design.Icon(output.Kind is "table" or "csv" ? "table" : output.Kind == "image" ? "image" : "doc", 14, design.ColorText("clay")) });
                var name = design.Text(output.Name, 12); name.TextWrapping = TextWrapping.NoWrap; name.TextTrimming = TextTrimming.CharacterEllipsis; name.VerticalAlignment = VerticalAlignment.Center; Grid.SetColumn(name, 1); row.Children.Add(name);
                if (output.Superseded) { var updated = design.Text(design.T("artifact.updated"), 10); updated.Foreground = design.Brush("text-faint"); updated.VerticalAlignment = VerticalAlignment.Center; Grid.SetColumn(updated, 2); row.Children.Add(updated); }
                var button = new Button { Content = row, IsEnabled = !output.Superseded, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch };
                design.QuietButton(button); button.MinHeight = 0; button.Padding = new(6, 4, 6, 4); button.CornerRadius = new(6); button.Background = design.Brush("bg-elev");
                ToolTipService.SetToolTip(button, output.Path ?? output.Name); AutomationProperties.SetName(button, output.Name); AutomationProperties.SetAutomationId(button, "generated-output");
                button.Click += (_, _) => open(output); content.Children.Add(button);
            }
            return content;
        }
        UIElement Toggle(string name, FrameworkElement children, string path, bool top)
        {
            var panel = new StackPanel { Spacing = top ? 6 : 0 };
            var header = new StackPanel { Orientation = Orientation.Horizontal, Spacing = top ? 5 : 6 };
            var arrow = design.Icon("chevron-right", 15); arrow.RenderTransformOrigin = new(.5, .5); header.Children.Add(arrow);
            var labelText = design.Text(name, top ? 10 : 12); labelText.Foreground = design.Brush(top ? "text-faint" : "text-muted");
            if (top) { labelText.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; labelText.CharacterSpacing = 50; }
            header.Children.Add(labelText);
            var button = new Button { Content = header, HorizontalAlignment = top ? HorizontalAlignment.Left : HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Left };
            design.QuietButton(button); button.MinHeight = 0; button.Padding = top ? new(0, 3, 0, 3) : new(6); AutomationProperties.SetName(button, name);
            void Show() { var expanded = choices.TryGetValue(path, out var value) && value; children.Visibility = expanded ? Visibility.Visible : Visibility.Collapsed; arrow.RenderTransform = new RotateTransform { Angle = expanded ? 90 : 0 }; AutomationProperties.SetItemStatus(button, expanded ? design.L("已展开", "Expanded") : design.L("已折叠", "Collapsed")); }
            button.Click += (_, _) => { choices[path] = !(choices.TryGetValue(path, out var value) && value); Show(); };
            Show(); panel.Children.Add(button); panel.Children.Add(children); return panel;
        }
    }
}
