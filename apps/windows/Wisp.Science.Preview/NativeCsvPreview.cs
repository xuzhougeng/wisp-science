using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;
using Windows.ApplicationModel.DataTransfer;

namespace Wisp.Science.Preview;

internal static class NativeCsvPreview
{
    public static FrameworkElement Create(NativeCsvTable data, WispDesign design, double maxHeight, bool markdown = false)
    {
        var root = new Grid { RowSpacing = 8 };
        root.RowDefinitions.Add(new() { Height = GridLength.Auto }); root.RowDefinitions.Add(new());
        var actions = new NativeActionWrap { Gap = 8 };
        var copy = design.ToolButton(design.L("复制表格", "Copy table"), "copy");
        copy.IsEnabled = data.Headers.Length > 0;
        copy.Click += (_, _) => { var clipboard = new DataPackage(); clipboard.SetText(data.CopyText); Clipboard.SetContent(clipboard); };
        actions.Children.Add(copy);
        var summary = design.L($"{data.Rows.Length} 行 · {data.Columns} 列", $"{data.Rows.Length} rows · {data.Columns} columns");
        if (data.Rows.Length > NativeCsvTable.VisibleRows || data.Columns > NativeCsvTable.VisibleColumns)
            summary += design.L("（仅显示前 500 行、64 列）", " (showing at most 500 rows, 64 columns)");
        if (data.Truncated) summary += design.L(" · 内容已截断", " · Truncated preview");
        actions.Children.Add(design.Text(summary, 12)); root.Children.Add(actions);
        var table = new Grid(); var columns = Math.Min(data.Columns, NativeCsvTable.VisibleColumns);
        for (var column = 0; column < columns; column++) table.ColumnDefinitions.Add(new() { Width = markdown ? new GridLength(1, GridUnitType.Star) : GridLength.Auto });
        var rows = new[] { data.Headers }.Concat(data.Rows.Take(NativeCsvTable.VisibleRows)).ToArray();
        for (var row = 0; row < rows.Length; row++)
        {
            table.RowDefinitions.Add(new() { Height = GridLength.Auto });
            for (var column = 0; column < columns; column++)
            {
                var text = design.Text(column < rows[row].Length ? rows[row][column] : "", 12);
                text.MaxWidth = markdown ? double.PositiveInfinity : 280; text.MinWidth = 70;
                if (markdown) TranscriptView.GeneratedCell(text, column < rows[row].Length ? rows[row][column] : "", design);
                if (row == 0) text.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
                var border = new Border { Child = text, Padding = new(12, 8, 12, 8), BorderBrush = design.Brush("border"),
                    BorderThickness = new(0, 0, 1, 1), Background = design.Brush(row == 0 ? "bg-sunken" : "bg-elev") };
                Grid.SetRow(border, row); Grid.SetColumn(border, column); table.Children.Add(border);
            }
        }
        AutomationProperties.SetName(table, design.L("CSV 表格预览", "CSV table preview"));
        var scroll = new ScrollViewer { Content = table, MaxHeight = Math.Max(60, maxHeight - 45),
            HorizontalScrollBarVisibility = ScrollBarVisibility.Auto, HorizontalScrollMode = ScrollMode.Enabled,
            VerticalScrollBarVisibility = ScrollBarVisibility.Auto };
        if (markdown)
        {
            table.Width = Math.Max(1, columns * 96);
            scroll.SizeChanged += (_, e) => table.Width = Math.Max(e.NewSize.Width, columns * 96);
        }
        Grid.SetRow(scroll, 1); root.Children.Add(scroll); return root;
    }
}
