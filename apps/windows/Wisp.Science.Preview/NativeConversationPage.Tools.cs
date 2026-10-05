using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Windows.ApplicationModel.DataTransfer;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class NativeConversationPage
{
    // Enable multiline before assigning Text: WinUI otherwise truncates at
    // the first newline during initialization, even for read-only controls.
    private TextBox ToolText(string value) => new() { AcceptsReturn = true, Text = value, IsReadOnly = true, TextWrapping = TextWrapping.Wrap,
        FontFamily = design.Font(true), FontSize = design.FontSize(12, true), MaxHeight = 320 };
    private Expander RawToolDetails(ConversationItem item)
    {
        var body = new StackPanel { Spacing = 6 };
        if (!string.IsNullOrEmpty(item.Input)) { body.Children.Add(design.Text("输入", 12)); body.Children.Add(ToolText(item.Input)); }
        body.Children.Add(design.Text("输出", 12)); body.Children.Add(ToolText(item.Text));
        if (!string.IsNullOrEmpty(item.Locations)) { body.Children.Add(design.Text("位置", 12)); body.Children.Add(ToolText(item.Locations)); }
        var copy = MessageAction("复制原始输出");
        copy.Click += (_, _) => { try { var data = new DataPackage(); data.SetText(item.Text); Clipboard.SetContent(data); } catch { } };
        body.Children.Add(copy);
        return new Expander { Header = "原始输入与输出", Content = body, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch };
    }
    private bool RenderAcpTool(StackPanel card, ConversationItem item)
    {
        if (item.Role != "acp_tool") return false;
        var sections = NativeStructuredTool.Sections(item);
        var body = new StackPanel { Spacing = 8 };
        foreach (var section in sections)
        {
            body.Children.Add(design.Text(section.Label, 12)); body.Children.Add(ToolText(section.Text));
        }
        body.Children.Add(RawToolDetails(item));
        card.Children.Add(new Expander { Header = NativeToolPresentation.Heading(item) + (string.IsNullOrWhiteSpace(item.Kind) ? "" : " · " + item.Kind),
            Content = body, IsExpanded = NativeToolPresentation.RequiresAttention(item) || item.Status is "pending" or "in_progress" or "running",
            HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch });
        return true;
    }
}
