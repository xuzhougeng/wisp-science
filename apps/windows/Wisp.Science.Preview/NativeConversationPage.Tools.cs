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
        FontFamily = design.Font(true), FontSize = design.FontSize(12, true), MaxHeight = 220,
        Background = design.Brush("bg-sunken"), BorderThickness = new(0) };
    private Expander RawToolDetails(ConversationItem item)
    {
        var body = new StackPanel { Spacing = 6 };
        if (!string.IsNullOrEmpty(item.Input)) { body.Children.Add(design.Text(design.L("输入", "Input"), 12)); body.Children.Add(ToolText(item.Input)); }
        body.Children.Add(design.Text(design.L("输出", "Output"), 12)); body.Children.Add(ToolText(item.Text));
        if (!string.IsNullOrEmpty(item.Locations)) { body.Children.Add(design.Text(design.L("位置", "Location"), 12)); body.Children.Add(ToolText(item.Locations)); }
        var copy = MessageAction(design.T("tool.copy_output"));
        copy.Click += (_, _) => { try { var data = new DataPackage(); data.SetText(item.Text); Clipboard.SetContent(data); } catch { } };
        body.Children.Add(copy);
        return new Expander { Header = design.L("原始输入与输出", "Raw input and output"), Content = body, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch };
    }
    private bool RenderAcpTool(StackPanel card, ConversationItem item, bool live)
    {
        if (item.Role != "acp_tool") return false;
        var body = new StackPanel { Spacing = 8 };
        var text = NativeToolPresentation.AcpBody(item);
        if (text.Length > 0) body.Children.Add(ToolText(text));
        card.Children.Add(ToolStep(item, body, live));
        return true;
    }
}
