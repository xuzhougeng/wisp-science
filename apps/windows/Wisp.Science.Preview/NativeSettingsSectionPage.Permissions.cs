using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed partial class NativeSettingsSectionPage
{
    private bool approvalAdvanced;
    private readonly Dictionary<string, ToggleButton> approvalModeButtons = [];
    private TextBlock PermissionText(string text, double size, string color = "text", bool medium = false)
    {
        var block = Design.Text(text, size); block.Foreground = Design.Brush(color);
        block.LineHeight = Design.FontSize(size) * (size == 14 ? 1.35 : 1.45);
        block.LineStackingStrategy = LineStackingStrategy.BlockLineHeight;
        if (medium) block.FontWeight = Microsoft.UI.Text.FontWeights.Medium;
        return block;
    }
    private async Task PermissionAction(Func<Task<bool>> action, string? focusMode = null, FocusState focus = FocusState.Programmatic)
    {
        await action();
        if (model.Closed) return;
        Render();
        if (focusMode != null && approvalModeButtons.TryGetValue(focusMode, out var button)) button.Focus(focus);
    }
    private Grid PermissionRow(FrameworkElement left, FrameworkElement right, double minimumCopyWidth)
    {
        var row = new Grid { ColumnSpacing = 12 };
        row.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); row.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        row.RowDefinitions.Add(new() { Height = GridLength.Auto }); row.RowDefinitions.Add(new() { Height = GridLength.Auto });
        left.VerticalAlignment = right.VerticalAlignment = VerticalAlignment.Center;
        row.Children.Add(left); Grid.SetColumn(right, 1); row.Children.Add(right);
        void Arrange()
        {
            var stacked = row.ActualWidth < minimumCopyWidth + right.DesiredSize.Width + 12;
            Grid.SetColumnSpan(left, stacked ? 2 : 1); Grid.SetColumn(right, stacked ? 0 : 1); Grid.SetRow(right, stacked ? 1 : 0);
            Grid.SetColumnSpan(right, stacked ? 2 : 1); row.RowSpacing = stacked ? 12 : 0;
            right.HorizontalAlignment = stacked ? HorizontalAlignment.Left : HorizontalAlignment.Right;
        }
        row.Loaded += (_, _) => Arrange(); row.SizeChanged += (_, _) => Arrange(); return row;
    }
    private Border ApprovalModes(params string[] modes)
    {
        var buttons = new NativeActionWrap { Gap = 0 };
        for (var i = 0; i < modes.Length; i++)
        {
            var mode = modes[i]; var selected = model.ApprovalScope == mode;
            var fill = selected ? mode switch { "ask" => "ok", "auto" => "warn", _ => "err" } : "bg-input";
            var button = new ToggleButton
            {
                Content = PermissionText(T("permissions.mode." + mode), 13, selected ? "on-clay" : "text-faint"),
                IsChecked = selected, IsEnabled = model.ApprovalScopeAvailable, MinHeight = 36, MinWidth = 0,
                Padding = new Thickness(14, 6, 14, 6), BorderThickness = new Thickness(i == 0 ? 0 : 1, 0, 0, 0),
                BorderBrush = Design.Brush("border-strong"), Background = Design.Brush(fill),
                CornerRadius = new CornerRadius(i == 0 ? 7 : 0, i == modes.Length - 1 ? 7 : 0, i == modes.Length - 1 ? 7 : 0, i == 0 ? 7 : 0)
            };
            Design.BindTypography(button, size: 13); AutomationProperties.SetName(button, T("permissions.mode." + mode));
            foreach (var state in new[] { "Checked", "CheckedPointerOver", "CheckedPressed", "PointerOver", "Pressed" })
            {
                button.Resources["ToggleButtonBackground" + state] = new SolidColorBrush(Design.Brush(fill).Color);
                button.Resources["ToggleButtonBorderBrush" + state] = new SolidColorBrush(Design.Brush("border-strong").Color);
            }
            approvalModeButtons[mode] = button;
            button.Click += async (_, _) =>
            {
                var focus = button.FocusState == FocusState.Keyboard ? FocusState.Keyboard : FocusState.Programmatic;
                button.IsChecked = selected; await PermissionAction(() => model.SetApprovalScopeAsync(mode), mode, focus);
            };
            buttons.Children.Add(button);
        }
        var group = new Border { Child = buttons, BorderBrush = Design.Brush("border-strong"), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(8) };
        AutomationProperties.SetName(group, T("permissions.mode")); return group;
    }
    private void Permissions()
    {
        approvalModeButtons.Clear();
        Body.Padding = new Thickness(24, 14, 24, 22); Body.Spacing = 0; Form.Spacing = 12;
        var grants = model.ApprovalGrants;
        var count = new Border { Child = PermissionText($"{T("settings.nav.permissions")} ({grants.Length})", 13, medium: true),
            Padding = new Thickness(12, 7, 12, 7), BorderBrush = Design.Brush("border-strong"), BorderThickness = new Thickness(1),
            Background = Design.Brush("bg-input"), CornerRadius = new CornerRadius(8), HorizontalAlignment = HorizontalAlignment.Left };
        var revokeAll = Button(T("permissions.revoke_all"), () => PermissionAction(model.RevokeAllApprovalsAsync));
        revokeAll.IsEnabled = grants.Length > 0; revokeAll.MinHeight = 0; revokeAll.Padding = new Thickness(10, 7, 10, 7);
        if (revokeAll.Content is TextBlock revokeText) Design.BindTypography(revokeText, 12);
        Form.Children.Add(PermissionRow(count, revokeAll, 135));

        var policy = new StackPanel { Spacing = 0 };
        var heading = new StackPanel { Spacing = 4 };
        heading.Children.Add(PermissionText(T("permissions.mode"), 14, medium: true));
        heading.Children.Add(PermissionText(T("permissions.mode.project"), 12, "text-faint"));
        policy.Children.Add(PermissionRow(heading, ApprovalModes("ask", "auto"), 300));
        var scope = model.ApprovalScope is "auto" or "full" ? model.ApprovalScope : "ask";
        var description = PermissionText("", 13, scope == "full" ? "err" : "text-muted");
        description.Inlines.Add(new Microsoft.UI.Xaml.Documents.Run { Text = T("permissions.mode." + scope) + "   ", FontWeight = Microsoft.UI.Text.FontWeights.Bold });
        description.Inlines.Add(new Microsoft.UI.Xaml.Documents.Run { Text = T("permissions.mode." + scope + ".desc") });
        description.Margin = new Thickness(0, 10, 0, 10); policy.Children.Add(description);

        var advanced = new StackPanel { Spacing = 8 };
        var details = PermissionRow(PermissionText(T("permissions.mode.full.desc"), 13, "text-muted"), ApprovalModes("full"), 240);
        details.Visibility = approvalAdvanced ? Visibility.Visible : Visibility.Collapsed;
        var toggle = new ToggleButton { IsChecked = approvalAdvanced, HorizontalAlignment = HorizontalAlignment.Left,
            Padding = new Thickness(0, 4, 0, 4), MinHeight = 0, MinWidth = 0, BorderThickness = new Thickness(0), Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent) };
        foreach (var state in new[] { "Checked", "CheckedPointerOver", "CheckedPressed", "PointerOver", "Pressed" })
        {
            toggle.Resources["ToggleButtonBackground" + state] = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
            toggle.Resources["ToggleButtonBorderBrush" + state] = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        }
        void AdvancedContent()
        {
            var content = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4 };
            content.Children.Add(Design.Icon(approvalAdvanced ? "chevron-down" : "chevron-right", 12, Design.ColorText("text-muted")));
            content.Children.Add(PermissionText(T("permissions.mode.advanced"), 13, "text-muted")); toggle.Content = content;
        }
        AdvancedContent(); AutomationProperties.SetName(toggle, T("permissions.mode.advanced"));
        toggle.Click += (_, _) => { approvalAdvanced = toggle.IsChecked == true; details.Visibility = approvalAdvanced ? Visibility.Visible : Visibility.Collapsed; AdvancedContent(); };
        advanced.Children.Add(toggle); advanced.Children.Add(details); policy.Children.Add(advanced);
        if (model.Error != null)
        {
            var error = PermissionText(model.Error, 13, "err"); error.Margin = new Thickness(0, 10, 0, 0);
            AutomationProperties.SetLiveSetting(error, Microsoft.UI.Xaml.Automation.Peers.AutomationLiveSetting.Assertive);
            policy.Children.Add(error);
        }
        Form.Children.Add(new Border { Child = policy, Padding = new Thickness(0, 14, 0, 14), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 1, 0, 1) });
        Form.Children.Add(PermissionText(T("permissions.note"), 12, "text-faint"));
        if (grants.Length == 0)
            Form.Children.Add(new Border { Child = PermissionText(T("permissions.empty"), 12), Padding = new Thickness(10, 8, 10, 8),
                BorderBrush = Design.Brush("border-strong"), BorderThickness = new Thickness(1), Background = Design.Brush("bg-input"), CornerRadius = new CornerRadius(8) });
        var list = new StackPanel { BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 1, 0, 0), Margin = new Thickness(0, 2, 0, 0) };
        for (var i = 0; i < grants.Length; i++)
        {
            var grant = grants[i]; var row = new Grid { ColumnSpacing = 12, RowSpacing = 4, Padding = new Thickness(2, 13, 2, 13), MinHeight = 48,
                BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 0, 0, i < grants.Length - 1 ? 1 : 0), Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent) };
            row.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); row.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            row.RowDefinitions.Add(new() { Height = GridLength.Auto }); row.RowDefinitions.Add(new() { Height = GridLength.Auto });
            var title = PermissionText(S(grant, "label"), 14, medium: true); title.VerticalAlignment = VerticalAlignment.Center; row.Children.Add(title);
            var subtitle = PermissionText(S(grant, "kind") + " - " + S(grant, "target"), 12, "text-faint");
            subtitle.TextWrapping = TextWrapping.NoWrap; subtitle.TextTrimming = TextTrimming.CharacterEllipsis; ToolTipService.SetToolTip(subtitle, subtitle.Text);
            Grid.SetRow(subtitle, 1); Grid.SetColumnSpan(subtitle, 2); row.Children.Add(subtitle);
            var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6, VerticalAlignment = VerticalAlignment.Center };
            var scopeKey = S(grant, "scope") is "session" or "project" or "global" ? "permissions.scope." + S(grant, "scope") : "approval.scope.once";
            actions.Children.Add(new Border { Child = PermissionText(T(scopeKey), 11, "text-muted"), Background = Design.Brush("bg-sunken"),
                Padding = new Thickness(6, 2, 6, 2), CornerRadius = new CornerRadius(99), VerticalAlignment = VerticalAlignment.Center });
            var revoke = Design.ToolButton(T("permissions.revoke"), "close"); revoke.Width = revoke.Height = 28; revoke.MinHeight = revoke.MinWidth = 0;
            revoke.Padding = new Thickness(0); revoke.CornerRadius = new CornerRadius(6); revoke.Content = Design.Icon("close", 15, Design.ColorText("text-faint"));
            var hover = Design.Brush("err").Color; hover.A = 26; revoke.Resources["ButtonBackgroundPointerOver"] = new SolidColorBrush(hover);
            var hovered = false;
            void Reveal() => revoke.Opacity = hovered || revoke.FocusState != FocusState.Unfocused ? 1 : 0;
            row.PointerEntered += (_, _) => { hovered = true; Reveal(); }; row.PointerExited += (_, _) => { hovered = false; Reveal(); };
            revoke.GotFocus += (_, _) => Reveal(); revoke.LostFocus += (_, _) => Reveal();
            revoke.PointerEntered += (_, _) => revoke.Content = Design.Icon("close", 15, Design.ColorText("err"));
            revoke.PointerExited += (_, _) => revoke.Content = Design.Icon("close", 15, Design.ColorText("text-faint"));
            revoke.Click += async (_, _) => await PermissionAction(() => model.RevokeApprovalAsync(grant)); Reveal();
            actions.Children.Add(revoke); Grid.SetColumn(actions, 1); row.Children.Add(actions); list.Children.Add(row);
        }
        Form.Children.Add(list);
    }
}
