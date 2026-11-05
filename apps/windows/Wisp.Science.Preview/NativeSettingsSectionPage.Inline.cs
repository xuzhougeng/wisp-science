using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Automation;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

// Shared rows for preference pages. Collection/detail editors keep their own lifecycle.
internal sealed partial class NativeSettingsSectionPage
{
    private bool GeneralInlineFooter => section == "general" && model.InlinePreferences;
    private void MovePreferenceFooter(Panel target)
    {
        if (preferenceFooter.Parent is Panel previous) previous.Children.Remove(preferenceFooter);
        target.Children.Add(preferenceFooter);
        var inline = target != Footer;
        preferenceFooter.Margin = inline ? new Thickness(0, section is "pet" or "channels" ? 4 : 20, 0, 0) : new Thickness(0);
        preferenceFooter.Padding = inline ? new Thickness(0, section is "general" or "channels" ? 16 : 14, 0, 0) : new Thickness(0);
        preferenceFooter.BorderBrush = Design.Brush("border");
        preferenceFooter.BorderThickness = new Thickness(0, inline ? 1 : 0, 0, 0);
    }
    private StackPanel InlineCard(string title)
    {
        var panel = new StackPanel { Spacing = 0 };
        var heading = Design.Text(title, section == "session" ? 15 : 16); heading.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        heading.Margin = new Thickness(0, 0, 0, 16); panel.Children.Add(heading);
        var card = Design.Card(panel); card.Padding = new Thickness(24, 22, 24, 22);
        ResponsiveSettingsCard(card);
        Form.Children.Add(card); return panel;
    }
    private void ResponsiveSettingsCard(Border card)
    {
        void Resize() => card.Padding = (XamlRoot?.Size.Width ?? ActualWidth) <= 850
            ? new Thickness(16, 18, 16, 18) : new Thickness(24, 22, 24, 22);
        card.Loaded += (_, _) => Resize(); card.SizeChanged += (_, _) => Resize();
    }
    private void InlineHeading(StackPanel target, string title)
    {
        var heading = Design.Text(title, section == "session" ? 15 : 16); heading.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        target.Children.Add(new Border { Child = heading, BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 1, 0, 0),
            Margin = new Thickness(0, 20, 0, 16), Padding = new Thickness(0, 20, 0, 0) });
    }
    private void InlineRow(StackPanel target, string title, string? hint, Control control, bool compact = false, bool emphasized = false, bool divider = true, UIElement? details = null)
    {
        var general = section == "general";
        var sessionNumber = section == "session" && control is TextBox;
        // Only stacked controls need a second-row gap. Empty rows must not add
        // vertical space to every toggle and number field.
        var toggleRow = control is ToggleSwitch;
        var padding = section == "pet" && toggleRow ? 0 : section == "browser" || general && hint != null ? 14 : 12;
        var row = new Grid { ColumnSpacing = general ? hint == null ? 12 : 16 : 24, RowSpacing = 0,
            MinHeight = toggleRow && !general ? section == "session" ? 64 : section == "pet" ? 65 : 66 : 0, Padding = new Thickness(0, padding, 0, padding) };
        row.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        row.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        row.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) }); row.RowDefinitions.Add(new() { Height = GridLength.Auto });
        var copy = new StackPanel { Spacing = toggleRow && !general ? 3 : 5, VerticalAlignment = VerticalAlignment.Center };
        var label = Design.Text(title, general || section == "session" && control is TextBox ? 13 : 14); if (emphasized) label.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        else if (general && toggleRow || sessionNumber) label.FontWeight = Microsoft.UI.Text.FontWeights.Medium;
        if (general && control is ComboBox) label.FontWeight = Microsoft.UI.Text.FontWeights.Medium;
        if (general && toggleRow)
        {
            // WebView's 13px inline strong occupies its parent's 14px line box.
            Design.BindLineHeight(label, 14, 1.4);
            copy.Spacing = 4;
        }
        copy.Children.Add(label); if (!string.IsNullOrEmpty(hint))
        {
            var description = Mute(hint);
            if (general) { Design.BindTypography(description, 12); Design.BindLineHeight(description, 12, 1.6); }
            else if (section is "session" or "browser" or "pet")
            { Design.BindTypography(description, 12); description.Foreground = Design.Brush("text-faint"); }
            if (sessionNumber)
            {
                Design.BindLineHeight(description, 12, 1.45);
                description.FontWeight = Microsoft.UI.Text.FontWeights.Medium;
            }
            copy.Children.Add(description);
        }
        if (details != null)
        {
            copy.Children.Add(details);
            if (general && details is FrameworkElement detail)
                copy.SizeChanged += (_, e) => detail.Width = Math.Min(detail.MaxWidth, Math.Max(0, e.NewSize.Width));
        }
        row.Children.Add(copy); control.VerticalAlignment = VerticalAlignment.Center;
        AutomationProperties.SetName(control, title); Design.BindTypography(control, size: general ? 13 : null);
        if (general && control is ComboBox) FocusLabeledField(row, label, control);
        if (sessionNumber)
        {
            // The WebView wraps both caption and help in the input's label.
            // Keep keyboard editing and disabled-state handling on TextBox.
            AutomationProperties.SetLabeledBy(control, label);
            row.Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
            row.Tapped += (_, _) => { if (control.IsEnabled) control.Focus(FocusState.Pointer); };
        }
        Grid.SetColumn(control, 1); row.Children.Add(control);
        void Arrange()
        {
            var width = row.ActualWidth;
            var stacked = !compact && !general && width < 520;
            row.RowSpacing = stacked ? 8 : 0;
            row.RowDefinitions[0].Height = stacked ? GridLength.Auto : new GridLength(1, GridUnitType.Star);
            if (section == "session" && control is TextBox)
            {
                var narrow = NativeSettingsLayout.SessionNumberWidth(XamlRoot?.Size.Width ?? ActualWidth) == 90;
                control.Width = narrow ? 90 : 112; row.ColumnSpacing = narrow ? 12 : 24; copy.Spacing = narrow ? 8 : 5;
            }
            Grid.SetColumn(control, stacked ? 0 : 1); Grid.SetRow(control, stacked ? 1 : 0);
            Grid.SetColumnSpan(copy, stacked ? 2 : 1); Grid.SetColumnSpan(control, stacked ? 2 : 1);
            control.MaxWidth = Math.Max(0, stacked ? width : width * (general ? .68 : .64));
            control.HorizontalAlignment = stacked ? HorizontalAlignment.Stretch : HorizontalAlignment.Right;
        }
        row.SizeChanged += (_, _) => Arrange(); row.Loaded += (_, _) => Arrange();
        target.Children.Add(new Border { Child = row, BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 0, 0, divider ? 1 : 0) });
    }
    private ToggleSwitch InlineToggle(StackPanel target, string title, string hint, bool value, Action<bool> changed, bool divider = true, UIElement? details = null)
    {
        var toggle = new ToggleSwitch { IsOn = value, OnContent = "", OffContent = "", MinWidth = 0 };
        StylePreferenceToggle(toggle);
        toggle.Toggled += (_, _) => { changed(toggle.IsOn); LockConfirmation(); };
        InlineRow(target, title, hint, toggle, compact: true, emphasized: section is "session" or "pet", divider: divider, details: details);
        return toggle;
    }
    private void StylePreferenceToggle(ToggleSwitch toggle)
    {
        PreferenceToggleSwitch.Apply(toggle, Design);
    }
    private ComboBox InlineChoice(StackPanel target, string title, string value, (string Id, string Label)[] options, Action<string> changed)
    {
        // WinUI's template already reserves the chevron's right inset.
        var combo = new ComboBox { CornerRadius = new CornerRadius(section == "general" ? 10 : 8), Padding = new Thickness(11, 8, section == "general" ? 0 : 11, 8),
            Background = Design.Brush("bg-input"), BorderBrush = Design.Brush("border-strong") };
        foreach (var option in options)
        {
            var item = new ComboBoxItem { Content = option.Label, Tag = option.Id };
            if (section == "general") Design.BindTypography(item, size: 13);
            combo.Items.Add(item);
        }
        combo.SelectedItem = combo.Items.Cast<ComboBoxItem>().FirstOrDefault(i => (string)i.Tag == value);
        combo.SelectionChanged += (_, _) => { if (combo.SelectedItem is ComboBoxItem item) { changed((string)item.Tag); LockConfirmation(); } };
        choices.Add(combo); InlineRow(target, title, null, combo, divider: false);
        if (section == "general")
        {
            combo.FontWeight = Microsoft.UI.Text.FontWeights.Medium;
            PreferencePickerAppearance.SizeToOptions(combo, Design);
        }
        return combo;
    }
    private void InlineText(StackPanel target, JsonObject draft, string key, string title, string placeholder = "")
    {
        var group = new StackPanel { Spacing = 8, Padding = new Thickness(0, 12, 0, 12) };
        var label = Design.Text(title, section == "general" ? 13 : 14);
        group.Children.Add(label);
        var text = new TextBox { Text = S(draft, key), PlaceholderText = placeholder, CornerRadius = new CornerRadius(8), Padding = new Thickness(11, 8, 11, 8),
            Background = Design.Brush("bg-input"), BorderBrush = Design.Brush("border-strong"), BorderThickness = new Thickness(1) };
        Design.BindTypography(text, code: section == "general", size: section == "general" ? 12 : null); AutomationProperties.SetName(text, title);
        if (section == "general")
        {
            label.FontWeight = text.FontWeight = Microsoft.UI.Text.FontWeights.Medium;
            text.CornerRadius = new(10);
            PreferenceFieldAppearance.UsePlainTextField(text);
            FocusLabeledField(group, label, text);
        }
        textMenus.Register(text);
        text.TextChanged += (_, _) => { draft[key] = text.Text; LockConfirmation(); };
        group.Children.Add(text); target.Children.Add(group);
    }
    // Match an HTML wrapping label: clicking its text or blank space focuses
    // the field. Leave clicks inside the field to its own caret/picker logic.
    private static void FocusLabeledField(Panel group, TextBlock label, Control field)
    {
        AutomationProperties.SetLabeledBy(field, label);
        group.Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        group.Tapped += (_, e) =>
        {
            if (!field.IsEnabled) return;
            for (var element = e.OriginalSource as DependencyObject; element != null && element != group; element = VisualTreeHelper.GetParent(element))
                if (element == field) return;
            field.Focus(FocusState.Pointer); e.Handled = true;
        };
    }
    private Action<bool> SessionInteger(StackPanel target, JsonObject draft, string key, string title, string hint, bool dependent = false)
    {
        var field = new SessionNumberDraft(draft, key);
        var input = new TextBox { Text = field.Text, Width = 112, CornerRadius = new CornerRadius(8),
            MinHeight = 36, FontWeight = Microsoft.UI.Text.FontWeights.Medium,
            Padding = new Thickness(11, 9, 11, 9), Background = Design.Brush("bg-input"), BorderBrush = Design.Brush("border-strong") };
        input.Resources["TextControlBackgroundDisabled"] = Design.Brush("bg-input");
        input.Resources["TextControlForegroundDisabled"] = Design.Brush("text");
        input.Resources["TextControlBorderBrushDisabled"] = Design.Brush("border-strong");
        AutomationProperties.SetHelpText(input, hint);
        textMenus.Register(input); PreferenceNumberBox.ApplyUnsigned(input, Design, field.Minimum, field.Maximum);
        input.TextChanged += (_, _) =>
        {
            field.Edit(input.Text);
            if (field.Invalid) invalidFields.Add(title); else invalidFields.Remove(title);
            LockConfirmation();
        };
        var rows = dependent ? new StackPanel() : target;
        InlineRow(rows, title, hint, input, compact: true, divider: false);
        if (dependent) target.Children.Add(new Border { Child = rows, Margin = new Thickness(16, 8, 0, 0), Padding = new Thickness(20, 0, 0, 0),
            BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(2, 0, 0, 0) });
        // Only the owning preference changes this state. A busy page or an open
        // discard prompt can temporarily disable controls without discarding text.
        return enabled =>
        {
            field.SetEnabled(enabled);
            if (!enabled) { invalidFields.Remove(title); if (input.Text != field.Text) input.Text = field.Text; }
            input.IsEnabled = enabled;
            input.Opacity = enabled ? 1 : .55;
        };
    }
    private bool showInlineValidation;
    private void InlineSaveActions()
    {
        editorActions.HorizontalAlignment = HorizontalAlignment.Right;
        editorActions.Children.Add(Button(T("settings.cancel"), () =>
        {
            void Cancel()
            {
                if (!model.CancelPreferences()) return;
                RestoreLanguage(); invalidFields.Clear(); localEnvironment?.Cancel(); base.HandleEscape();
            }
            if (localEnvironment?.HasChanges == true)
                Confirm(L("放弃尚未保存的路径修改？", "Discard unsaved path changes?"), Cancel);
            else Cancel();
            return Task.CompletedTask;
        }));
        var save = Button(T("settings.save"), async () =>
        {
            if (invalidFields.Count > 0) { showInlineValidation = true; LockConfirmation(); return; }
            async Task SaveAndClose()
            {
                if (await model.SaveAsync() && !model.Closed) base.HandleEscape();
            }
            if (localEnvironment?.HasChanges == true)
                Confirm(L("保存偏好设置并放弃未保存的路径修改？", "Save preferences and discard unsaved path changes?"), () => _ = SaveAndClose());
            else await SaveAndClose();
        });
        Design.ActionButton(save, true); editorActions.Children.Add(save);
        editorActions.Spacing = section == "general" ? 10 : 8;
        Design.BindTypography(saveFeedback, 12); saveFeedback.Foreground = Design.Brush("text-muted");
        foreach (var button in editorActions.Children.OfType<Button>())
        { StyleGeneralButton(button, primary: button == save); button.MinWidth = 84; }
    }
    private void StyleGeneralButton(Button button, bool primary = false, bool compact = false)
    {
        Design.ActionButton(button, primary);
        button.MinHeight = 0;
        button.Padding = compact ? new Thickness(12, 7, 12, 7) : new Thickness(16, 9, 16, 9);
        button.Background = Design.Brush(primary ? "clay" : "bg-input");
        button.BorderBrush = Design.Brush(primary ? "clay" : "border-strong");
        button.Foreground = Design.Brush(primary ? "on-clay" : "text");
        if (button.Content is TextBlock label)
        {
            Design.BindTypography(label, compact ? 12 : 13.5); label.Foreground = button.Foreground;
            if (primary) label.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        }
    }
}
