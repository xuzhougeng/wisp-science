using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class NativeConversationPage
{
    private FrameworkElement AssistantHeader(ConversationItem item)
    {
        var row = new NativeActionWrap { Gap = 7, RowGap = 4, VerticallyCentered = true };
        row.Children.Add(new Image { Source = new SvgImageSource(new Uri("ms-appx:///Assets/logo.svg")), Width = 15, Height = 15, Stretch = Stretch.Uniform });
        var brand = design.Text(design.T("chat.assistant").ToUpperInvariant(), 11);
        brand.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; brand.Foreground = design.Brush("text-muted"); brand.CharacterSpacing = 50;
        row.Children.Add(brand);
        if (!string.IsNullOrEmpty(item.ModelName))
        {
            var name = new TextBlock { Text = item.ModelName, FontFamily = design.Font(true), FontSize = design.FontSize(10.5),
                FontWeight = Microsoft.UI.Text.FontWeights.Medium, Foreground = design.Brush("text-faint"), TextWrapping = TextWrapping.Wrap };
            row.Children.Add(new Border { Child = name, MaxWidth = 360, Padding = new(7, 1, 7, 1), CornerRadius = new(99),
                Background = design.Brush("bg-elev"), BorderBrush = design.Brush("border"), BorderThickness = new(1) });
        }
        if (MessageTime(item, false) is { } time) row.Children.Add(time);
        AutomationProperties.SetAutomationId(row, "assistant-message-header"); return row;
    }
    private TextBlock? MessageTime(ConversationItem item, bool user)
    {
        var value = NativeMessagePresentation.Time(item.Timestamp, design.Localization.Locale, DateTimeOffset.Now);
        if (value == null) return null;
        var text = new TextBlock { Text = value.Compact, FontFamily = design.Font(true), FontSize = design.FontSize(10.5),
            Foreground = design.Brush("text-faint"), FontWeight = Microsoft.UI.Text.FontWeights.Medium };
        ToolTipService.SetToolTip(text, design.Localization.Format(user ? "msg.sent_at" : "msg.replied_at", ("time", value.Full)));
        AutomationProperties.SetName(text, design.Localization.Format(user ? "msg.sent_at" : "msg.replied_at", ("time", value.Full)));
        return text;
    }
    private FrameworkElement UserMessageBody(ConversationItem item, bool raw)
    {
        var presentation = raw ? new NativeUserMessage(item.Text, item.Text, false, [], []) : NativeMessagePresentation.User(item.Text, item.Attachments);
        var content = new StackPanel { Spacing = 8, HorizontalAlignment = HorizontalAlignment.Right };
        if (presentation.Attachments.Length > 0 || presentation.References.Length > 0)
        {
            foreach (var images in new[] { true, false })
            {
                var files = new NativeActionWrap { Gap = images ? 8 : 7, RowGap = images ? 8 : 7, RightAligned = true };
                foreach (var path in presentation.Attachments.Where(path => NativeAttachmentPresentation.Image(path) == images))
                {
                    var name = Path.GetFileName(path.Replace('\\', '/'));
                    var canOpen = !model.ExampleMode && model.CanPreviewAttachment(path);
                    FrameworkElement card = images ? UserImage(path, name, canOpen) : UserReference("doc", name, "file");
                    // Keep read-only/unavailable attachments legible, without implying a working preview.
                    if (!canOpen) { files.Children.Add(card); ToolTipService.SetToolTip(card, path); }
                    else
                    {
                        var button = new Button { Content = card, Padding = new(0), Background = design.Brush("bg-elev"), BorderThickness = new(0), CornerRadius = new(8) };
                        if (images && card is Grid picture && picture.Children.LastOrDefault() is Border caption)
                        { button.GotFocus += (_, _) => caption.Opacity = 1; button.LostFocus += (_, _) => caption.Opacity = 0; }
                        AutomationProperties.SetName(button, path); ToolTipService.SetToolTip(button, path); button.Click += async (_, _) => await OpenAttachmentAsync(path, name); files.Children.Add(button);
                    }
                }
                if (files.Children.Count > 0) content.Children.Add(files);
            }
            var refs = new NativeActionWrap { Gap = 7, RowGap = 7, RightAligned = true };
            foreach (var reference in presentation.References)
                refs.Children.Add(UserReference(reference.Kind switch { "skill" => "skill", "workflow" => "branch", "session" => "chat", "project" => "folder", "context" => "server", "runtime" => "terminal", _ => "doc" }, reference.Label, reference.Kind));
            if (refs.Children.Count > 0) content.Children.Add(refs);
        }
        if (presentation.Body.Length > 0)
        {
            var text = new TextBlock { Text = presentation.ShortBody, FontFamily = design.Font(), FontSize = Math.Max(.01, design.Typography.UiSize + .5),
                Foreground = design.Brush("text"), TextWrapping = TextWrapping.Wrap, IsTextSelectionEnabled = true,
                LineHeight = Math.Max(.01, design.Typography.UiSize + .5) * 1.55, LineStackingStrategy = LineStackingStrategy.MaxHeight };
            AutomationProperties.SetAutomationId(text, "user-message-body");
            content.Children.Add(new Border { Child = text, Padding = new(15, 10, 15, 10), Background = design.Brush("bg-panel"),
                BorderBrush = design.Brush("border"), BorderThickness = new(1), CornerRadius = new(18, 18, 6, 18), HorizontalAlignment = HorizontalAlignment.Right });
            if (presentation.IsLong)
            {
                var expanded = false; var toggle = MessageAction(design.T("msg.show_all")); toggle.HorizontalAlignment = HorizontalAlignment.Right;
                toggle.Click += (_, _) => { expanded = !expanded; text.Text = expanded ? presentation.Body : presentation.ShortBody; var label = design.T(expanded ? "msg.show_less" : "msg.show_all"); toggle.Content = label; AutomationProperties.SetName(toggle, label); };
                content.Children.Add(toggle);
            }
        }
        if (!raw && MessageTime(item, true) is { } time) { time.HorizontalAlignment = HorizontalAlignment.Right; time.Margin = new(0, -2, 4, -3); content.Children.Add(time); }
        return content;
    }
    private Border UserReference(string icon, string label, string kind)
    {
        var row = new Grid { ColumnSpacing = 9, MinHeight = 34 };
        row.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); row.ColumnDefinitions.Add(new());
        var tone = kind is "session" or "context" ? "info" : kind == "runtime" ? "ok" : "clay";
        var color = kind == "skill" ? Windows.UI.Color.FromArgb(255, 118, 85, 199) : design.Brush(tone).Color;
        var soft = color; soft.A = 31;
        row.Children.Add(new Border { Child = design.Icon(icon, 18, $"#{color.R:X2}{color.G:X2}{color.B:X2}"), Width = 34, Height = 34,
            CornerRadius = new(4), Background = new SolidColorBrush(soft), VerticalAlignment = VerticalAlignment.Center });
        var words = new StackPanel { Spacing = 2, VerticalAlignment = VerticalAlignment.Center };
        var name = design.Text(label, 12.5); name.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        name.TextWrapping = TextWrapping.NoWrap; name.TextTrimming = TextTrimming.CharacterEllipsis;
        var meta = design.Text(design.T("attachment." + kind), 10.5); meta.Foreground = design.Brush("text-faint"); words.Children.Add(name); words.Children.Add(meta);
        Grid.SetColumn(words, 1); row.Children.Add(words);
        if (kind == "file") { row.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); var next = design.Icon("chevron-right", 14, design.ColorText("text-faint")); Grid.SetColumn(next, 2); row.Children.Add(next); }
        return new Border { Child = row, Width = kind == "file" ? 286 : 242, Padding = new(10, kind == "file" ? 9 : 8, 10, kind == "file" ? 9 : 8), CornerRadius = new(8), Background = design.Brush("bg-elev"), BorderBrush = design.Brush(kind == "file" ? "border-strong" : "border"), BorderThickness = new(1) };
    }
    private FrameworkElement UserImage(string path, string name, bool canRead)
    {
        var view = new Grid { Width = 220, Height = 154, Background = design.Brush("bg-sunken"), CornerRadius = new(8) };
        view.Children.Add(canRead ? new NativeAttachmentThumbnail(model, path, design, 220, 154) : design.Icon("image", 24));
        var label = design.Text(name, 10.5); label.Foreground = new SolidColorBrush(Microsoft.UI.Colors.White);
        label.TextWrapping = TextWrapping.NoWrap; label.TextTrimming = TextTrimming.CharacterEllipsis;
        var caption = new Border { Child = label, Padding = new(7, 4, 7, 4), Margin = new(7), CornerRadius = new(4), Background = new SolidColorBrush(Windows.UI.Color.FromArgb(168, 15, 18, 20)), VerticalAlignment = VerticalAlignment.Bottom, Opacity = 0 };
        view.Children.Add(caption); view.PointerEntered += (_, _) => caption.Opacity = 1; view.PointerExited += (_, _) => caption.Opacity = 0;
        view.GotFocus += (_, _) => caption.Opacity = 1; view.LostFocus += (_, _) => caption.Opacity = 0;
        void Fit()
        {
            view.Width = NativeMessagePresentation.ImageWidth(XamlRoot?.Size.Width ?? ActualWidth);
            if (view.Children[0] is NativeAttachmentThumbnail thumbnail) thumbnail.Width = view.Width;
        }
        SizeChangedEventHandler resize = (_, _) => Fit();
        view.Loaded += (_, _) => { SizeChanged -= resize; SizeChanged += resize; Fit(); };
        view.Unloaded += (_, _) => SizeChanged -= resize;
        return view;
    }
}
