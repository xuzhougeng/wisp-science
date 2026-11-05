using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

/// <summary>The same folder disclosure in normal and multi-select sidebars.</summary>
internal sealed class NativeSessionFolderHeader : Grid
{
    private readonly WispDesign design;
    private readonly Grid caret = new() { Width = 12, Height = 12 };
    public Button ToggleButton { get; }
    public NativeSessionFolderHeader(SessionSection section, WispDesign design, Action toggle,
        Action? rename, NativeSessionCheck? selection = null, FlyoutBase? menu = null, bool rail = false)
    {
        this.design = design;
        var title = section.DisplayTitle(design.Localization);
        var content = new Grid { ColumnSpacing = 6 };
        content.ColumnDefinitions.Add(new() { Width = new GridLength(12) });
        content.ColumnDefinitions.Add(new() { Width = new GridLength(15) });
        content.ColumnDefinitions.Add(new()); content.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        content.Children.Add(caret);
        if (selection == null)
        {
            var icon = design.Icon("folder", 15, design.ColorText("text-muted")); icon.Opacity = .8;
            Grid.SetColumn(icon, 1); content.Children.Add(icon);
        }
        var label = design.Text(title, 13); label.FontWeight = Microsoft.UI.Text.FontWeights.Medium;
        label.Foreground = design.Brush("text-muted"); label.TextWrapping = TextWrapping.NoWrap;
        label.TextTrimming = TextTrimming.CharacterEllipsis; label.VerticalAlignment = VerticalAlignment.Center;
        Grid.SetColumn(label, 2); content.Children.Add(label);
        var count = design.Text(section.Sessions.Length.ToString(), 11); count.Foreground = design.Brush("text-faint");
        count.VerticalAlignment = VerticalAlignment.Center; Grid.SetColumn(count, 3); content.Children.Add(count);
        ToggleButton = new Button { Content = content, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        design.QuietButton(ToggleButton); ToggleButton.MinWidth = ToggleButton.MinHeight = 0;
        ToggleButton.Padding = new(10, 6, selection == null ? 35 : 10, 6);
        if (rail)
        {
            ToggleButton.Content = design.Icon("folder", 15, design.ColorText("text-muted")); ToggleButton.MinHeight = 32;
            ToggleButton.Padding = new(2); ToggleButton.HorizontalContentAlignment = HorizontalAlignment.Center;
        }
        ToggleButton.Click += (_, _) => toggle(); Children.Add(ToggleButton);
        AutomationProperties.SetName(ToggleButton, title); ToolTipService.SetToolTip(ToggleButton, title);
        AutomationProperties.SetAutomationId(ToggleButton, "folder-" + section.FolderId);
        if (selection != null)
        {
            selection.HorizontalAlignment = HorizontalAlignment.Left; selection.VerticalAlignment = VerticalAlignment.Center;
            selection.Margin = new(rail ? 0 : 29, 0, 0, 0); Children.Add(selection);
        }
        if (rename != null)
        {
            ToggleButton.DoubleTapped += (_, e) => { e.Handled = true; rename(); };
            var edit = design.ToolButton(design.T(menu == null ? "ctx.rename_folder" : "folder.actions"), menu == null ? "edit" : "more"); design.QuietButton(edit);
            edit.Width = edit.Height = 25; edit.MinWidth = edit.MinHeight = 0; edit.Padding = new(4);
            edit.HorizontalAlignment = HorizontalAlignment.Right; edit.VerticalAlignment = VerticalAlignment.Center; edit.Margin = new(0, 0, 5, 0);
            if (menu == null) edit.Click += (_, _) => rename();
            else { edit.Flyout = menu; ToggleButton.ContextFlyout = menu; ContextFlyout = menu; Unloaded += (_, _) => menu.Hide(); }
            Children.Add(edit);
            if (rail) edit.Visibility = Visibility.Collapsed;
            var hovered = false; var focused = false;
            void Reveal() { edit.Opacity = hovered || focused ? 1 : 0; edit.IsHitTestVisible = hovered || focused; }
            PointerEntered += (_, _) => { hovered = true; Reveal(); }; PointerExited += (_, _) => { hovered = false; Reveal(); };
            GotFocus += (_, _) => { focused = true; Reveal(); }; LostFocus += (_, _) => { focused = false; Reveal(); }; Reveal();
        }
    }
    public void SetExpanded(bool expanded)
    {
        caret.Children.Clear(); caret.Children.Add(design.Icon(expanded ? "chevron-down" : "chevron-right", 12, design.ColorText("text-faint")));
        AutomationProperties.SetItemStatus(ToggleButton, expanded ? "expanded" : "collapsed");
    }
    public static TextBlock SectionTitle(SessionSection section, WispDesign design, bool first)
    {
        var label = design.Text(section.DisplayTitle(design.Localization), 11);
        label.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; label.Foreground = design.Brush("text-faint");
        label.Margin = new(10, first ? 4 : 12, 10, 4); return label;
    }
}
