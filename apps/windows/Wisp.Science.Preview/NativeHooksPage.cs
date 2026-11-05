using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed class NativeHooksPage : NativeActionPage
{
    private readonly NativeHooksModel model;
    private readonly SettingsTextMenus textMenus = new();
    private readonly Action openReviewer;
    private readonly StackPanel builtinHost = new() { Spacing = 0 };
    private readonly StackPanel commandHost = new() { Spacing = 14 };
    private readonly StackPanel projectHost = new() { Spacing = 10 };
    private readonly List<TextBox> numberInputs = [];
    private ComboBox? eventChoice;
    private TextBox? commandEditor;
    private Button? save, add;
    private Action? confirmation;
    public NativeHooksPage(INativeSettingsClient client, string? project, WispDesign design, Action close, Action openReviewer)
        : this(new NativeHooksModel(client, project, design.Localization), design, close, openReviewer) { _ = Load(); }
    private NativeHooksPage(NativeHooksModel model, WispDesign design, Action close, Action openReviewer) : base(design, design.T("settings.nav.hooks"), model, close)
    {
        this.model = model; this.openReviewer = openReviewer;
        UseSettingsLayout(reserveOverflowScrollbar: true); Body.Padding = new Thickness(24, 18, 24, 22); Body.Spacing = 0;
        Form.MaxWidth = double.PositiveInfinity; Form.Spacing = 14; Footer.Visibility = Visibility.Collapsed;
        builtinHost.SizeChanged += (_, _) =>
        {
            foreach (var input in numberInputs) input.Width = NativeSettingsLayout.SessionNumberWidth(XamlRoot?.Size.Width ?? ActualWidth);
        };
        model.Changed += RefreshControls;
        Design.TypographyChanged += ResizeCommandEditor;
    }
    private static string S(JsonNode? node, string key) => node?[key]?.GetValue<string>() ?? "";
    private string T(string key) => Design.T(key);
    private string L(string zh, string en) => Design.L(zh, en);
    private TextBlock Hint(string text)
    {
        var block = Design.Text(text, 12); block.Foreground = Design.Brush("text-faint");
        Design.BindLineHeight(block, 12, 1.45); return block;
    }
    private TextBlock Label(string text) { var block = Design.Text(text, 14); block.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; return block; }
    private TextBlock Group(string key, bool spaced = false)
    {
        var block = Design.Text(Design.Localization.Locale == "en" ? T(key).ToUpperInvariant() : T(key), 11);
        block.Foreground = Design.Brush("text-faint"); block.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        block.CharacterSpacing = 60; block.Margin = new Thickness(2, spaced ? 16 : 0, 2, spaced ? 6 : 0); return block;
    }
    private Border Status(string text) => new() { Child = Design.Text(text, 12), Padding = new Thickness(10, 8, 10, 8),
        Background = Design.Brush("bg-input"), BorderBrush = Design.Brush("border-strong"), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(10) };
    private Border Badge(string text)
    {
        var caption = Design.Text(text, 10); caption.Foreground = Design.Brush("text-muted");
        return new() { Child = caption, Padding = new Thickness(5, 1, 5, 1), CornerRadius = new CornerRadius(6), Background = Design.Brush("bg-sunken"), VerticalAlignment = VerticalAlignment.Center };
    }
    private void RefreshControls()
    {
        SetContentEnabled(model.Loaded && !model.Busy && confirmation == null);
        // A failed initial load must remain retryable.
        foreach (var action in HeaderActions.Children.OfType<Control>()) action.IsEnabled = !model.Busy && confirmation == null;
        if (save != null) { save.IsEnabled = model.CanSave && confirmation == null; save.Opacity = save.IsEnabled ? 1 : .5; }
        if (add != null) add.IsEnabled = !model.Busy && confirmation == null && !(model.Draft != null && model.EditingIndex == null);
    }
    public void RequestLeave(Action next)
    {
        if (model.Busy) return;
        if (model.HasChanges) Confirm(L("放弃尚未保存的修改？", "Discard unsaved changes?"), next); else next();
    }
    private void Confirm(string message, Action next)
    {
        confirmation = next; Notices.Children.Clear(); Notices.Children.Add(Hint(message));
        var actions = new NativeActionWrap();
        actions.Children.Add(Button(L("确认", "Confirm"), () => { var pending = confirmation; ClearConfirmation(); pending?.Invoke(); return Task.CompletedTask; }));
        actions.Children.Add(Button(T("settings.cancel"), () => { ClearConfirmation(); return Task.CompletedTask; }));
        Notices.Children.Add(actions); Notices.Visibility = Visibility.Visible; RefreshControls();
    }
    private void ClearConfirmation() { confirmation = null; Notices.Children.Clear(); Notices.Visibility = Visibility.Collapsed; RefreshControls(); }
    public override void HandleEscape()
    {
        if (textMenus.CloseTop()) return;
        if (eventChoice?.IsDropDownOpen == true) { eventChoice.IsDropDownOpen = false; return; }
        if (confirmation != null) { ClearConfirmation(); return; }
        if (model.Busy) return;
        if (model.Draft != null) { RequestLeave(() => { model.Discard(); Commands(); }); return; }
        base.HandleEscape();
    }
    private async Task Load() { await model.LoadAsync(); if (!model.Closed) Render(); }
    private void Render()
    {
        Form.Children.Clear(); HeaderActions.Children.Clear();
        SettingsHeading(T("settings.nav.hooks"));
        if (!model.Loaded)
        {
            var refresh = Design.ToolButton(L("刷新钩子", "Reload hooks"), "refresh"); refresh.Click += (_, _) => _ = Load(); HeaderActions.Children.Add(refresh);
            Form.Children.Add(Hint(L("钩子设置尚未载入，请点击刷新重试。", "Hooks could not be loaded. Use Reload to retry."))); RefreshControls(); return;
        }
        Form.Children.Add(Hint(T("hooks.intro")));
        Form.Children.Add(Group("hooks.builtin", true)); Form.Children.Add(builtinHost); Builtins();
        Form.Children.Add(commandHost); Commands(); Form.Children.Add(projectHost); ProjectHooks();
        Form.Children.Add(Hint(T("hooks.acp_note")));
        Design.ApplyTypography(Form); RefreshControls();
    }
    private Grid Row(FrameworkElement left, FrameworkElement right)
    {
        var row = new Grid { ColumnSpacing = 24 }; row.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); row.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        left.VerticalAlignment = right.VerticalAlignment = VerticalAlignment.Center;
        row.Children.Add(left); Grid.SetColumn(right, 1); row.Children.Add(right); return row;
    }
    private ToggleSwitch Toggle(string label, bool value, Func<bool, Task> changed)
    {
        var toggle = new ToggleSwitch { IsOn = value, OnContent = "", OffContent = "", MinWidth = 0 };
        PreferenceToggleSwitch.Apply(toggle, Design); AutomationProperties.SetName(toggle, label);
        toggle.Toggled += async (_, _) => await changed(toggle.IsOn); return toggle;
    }
    private void Builtins()
    {
        numberInputs.Clear(); builtinHost.Children.Clear();
        StackPanel Copy(string title, string description)
        {
            var copy = new StackPanel { Spacing = 3 }; var heading = new NativeActionWrap { Gap = 8 }; heading.Children.Add(Label(title)); heading.Children.Add(Badge("Stop"));
            copy.Children.Add(heading); copy.Children.Add(Hint(description)); return copy;
        }
        var review = Copy(T("composer.auto_review"), T("hooks.auto_review.desc"));
        var reviewerCaption = T("composer.reviewer_model") + ": " + model.ReviewerLabel;
        var reviewer = Design.ToolButton(reviewerCaption, "chevron-right");
        var reviewerContent = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 2 }; reviewerContent.Children.Add(Hint(reviewerCaption)); reviewerContent.Children.Add(Design.Icon("chevron-right", 14)); reviewer.Content = reviewerContent;
        reviewer.Padding = new Thickness(0); reviewer.MinHeight = 0; reviewer.HorizontalAlignment = HorizontalAlignment.Left;
        reviewer.Click += (_, _) => RequestLeave(openReviewer); review.Children.Add(reviewer);
        Border Builtin(FrameworkElement content, bool divider) => new() { Child = content, MinHeight = 64, Padding = new Thickness(0, 12, 0, 12), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 0, 0, divider ? 1 : 0) };
        builtinHost.Children.Add(Builtin(Row(review, Toggle(T("composer.auto_review"), model.AutoReview, async value => { await model.SetAutoReviewAsync(value); if (!model.Closed) Builtins(); })), true));
        var failure = Copy(T("composer.auto_failure_analysis"), T("hooks.failure_analysis.desc"));
        builtinHost.Children.Add(Builtin(Row(failure, Toggle(T("composer.auto_failure_analysis"), model.FailureAnalysis["enabled"]!.GetValue<bool>(), async value => { await model.SetFailureAsync("enabled", value ? 1 : 0); if (!model.Closed) Builtins(); })), model.FailureAnalysis["enabled"]!.GetValue<bool>()));
        if (model.FailureAnalysis["enabled"]!.GetValue<bool>())
        {
            Number(T("composer.failure_rate_threshold"), "failure_rate_threshold"); Number(T("composer.minimum_failures"), "minimum_failures");
        }
        Design.ApplyTypography(builtinHost); RefreshControls();
        void Number(string label, string field)
        {
            var saved = model.FailureAnalysis[field]!.GetValue<int>();
            var input = Input(label, saved.ToString(), _ => { });
            input.Width = NativeSettingsLayout.SessionNumberWidth(XamlRoot?.Size.Width ?? ActualWidth);
            numberInputs.Add(input);
            var validation = Hint(""); validation.Visibility = Visibility.Collapsed;
            input.LostFocus += async (_, _) =>
            {
                if (model.Closed || model.Busy || input.Text == saved.ToString()) return;
                if (!int.TryParse(input.Text, out var value) || value is < 1 or > 100) { validation.Text = L("请输入 1–100 的整数。", "Enter a whole number from 1 to 100."); validation.Visibility = Visibility.Visible; return; }
                if (await model.SetFailureAsync(field, value) && !model.Closed) Builtins();
            };
            var block = new StackPanel { Spacing = 4, Padding = new Thickness(0, 12, 0, 12) }; block.Children.Add(Row(Design.Text(label, 13), NativeDialogVisuals.Input(input, Design))); block.Children.Add(validation); builtinHost.Children.Add(block);
        }
    }
    private void OpenEditor(int? index = null) => RequestLeave(() => { model.Discard(); if (model.Edit(index)) Commands(); });
    private void Commands()
    {
        var returningFromEditor = eventChoice != null && model.Draft == null;
        commandHost.Children.Clear(); save = null; eventChoice = null; commandEditor = null;
        add = Design.ToolButton(T("hooks.add"), "plus", true); Design.ActionButton(add); add.Click += (_, _) => OpenEditor();
        add.Padding = new Thickness(12, 7, 12, 7); add.MinHeight = 0; add.CornerRadius = new CornerRadius(10);
        add.Background = Design.Brush("bg-input"); add.BorderBrush = Design.Brush("border-strong");
        var addContent = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 }; addContent.Children.Add(Design.Icon("plus", 14)); addContent.Children.Add(Design.Text(T("hooks.add"), 12.5)); add.Content = addContent;
        commandHost.Children.Add(Row(Group("hooks.custom"), add));
        if (model.Draft != null) Editor();
        if (model.Hooks.Count == 0)
            commandHost.Children.Add(Status(T("hooks.empty")));
        for (var i = 0; i < model.Hooks.Count; i++)
        {
            var index = i; var hook = (JsonObject)model.Hooks[index]!;
            var edit = Button(L("编辑钩子：", "Edit hook: ") + S(hook, "event"), () => { OpenEditor(index); return Task.CompletedTask; }); Design.QuietButton(edit); edit.Content = HookCopy(hook, false); edit.HorizontalContentAlignment = HorizontalAlignment.Stretch; edit.HorizontalAlignment = HorizontalAlignment.Stretch;
            var actions = new NativeActionWrap();
            var enabled = Toggle(T("hooks.enabled") + ": " + S(hook, "event"), hook["enabled"]!.GetValue<bool>(), value =>
            {
                RequestLeave(() => { model.Discard(); _ = ChangeHook(() => model.SetHookEnabledAsync(index, value)); }); return Task.CompletedTask;
            }); enabled.IsEnabled = model.Draft == null; actions.Children.Add(enabled);
            var remove = Design.ToolButton(T("hooks.remove"), "trash"); remove.Click += (_, _) => RequestLeave(() =>
            { model.Discard(); Commands(); Confirm(L("删除这条命令钩子？", "Remove this command hook?"), () => _ = ChangeHook(() => model.DeleteHookAsync(index))); }); actions.Children.Add(remove);
            commandHost.Children.Add(new Border { Child = Row(edit, actions), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 0, 0, 1), Padding = new Thickness(0, 8, 0, 8) });
        }
        if (returningFromEditor) add.Loaded += (_, _) => { if (!model.Closed && model.Draft == null) add.Focus(FocusState.Programmatic); };
        Design.ApplyTypography(commandHost); RefreshControls();
    }
    private async Task ChangeHook(Func<Task<bool>> action) { await action(); if (!model.Closed) Commands(); }
    private StackPanel HookCopy(JsonObject hook, bool full)
    {
        var body = new StackPanel { Spacing = 5 }; var title = new NativeActionWrap(); title.Children.Add(Label(S(hook, "event")));
        if (S(hook, "matcher").Length > 0) title.Children.Add(Badge(S(hook, "matcher")));
        if (hook["timeout"] != null) title.Children.Add(Badge(hook["timeout"] + "s")); body.Children.Add(title);
        var command = Hint(full ? S(hook, "command") : NativeHooksModel.CommandSummary(S(hook, "command"))); command.FontFamily = Design.Font(true); command.TextWrapping = full ? TextWrapping.Wrap : TextWrapping.NoWrap; command.TextTrimming = full ? TextTrimming.None : TextTrimming.CharacterEllipsis; body.Children.Add(command); return body;
    }
    private void Editor()
    {
        var draft = model.Draft!; var body = new StackPanel { Spacing = 10 }; var fields = new Grid { ColumnSpacing = 14, RowSpacing = 12 };
        fields.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); fields.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        eventChoice = new ComboBox { HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch,
            CornerRadius = new CornerRadius(10), Padding = new Thickness(11, 9, 11, 9), Background = Design.Brush("bg-input"), BorderBrush = Design.Brush("border-strong"), BorderThickness = new Thickness(1) };
        Design.BindTypography(eventChoice, size: 14);
        foreach (var value in NativeHooksModel.Events) eventChoice.Items.Add(value); eventChoice.SelectedItem = S(draft, "event"); AutomationProperties.SetName(eventChoice, T("hooks.event"));
        var eventGroup = Field(T("hooks.event"), eventChoice);
        var matcher = Input(T("hooks.matcher"), S(draft, "matcher"), value => model.SetDraftText("matcher", value)); matcher.PlaceholderText = "shell|write|edit";
        var matcherGroup = Field(T("hooks.matcher"), NativeDialogVisuals.Input(matcher, Design)); matcherGroup.Children.Add(Hint(T("hooks.matcher_hint")));
        var timeout = Input(T("hooks.timeout"), model.TimeoutText, model.SetTimeout); timeout.PlaceholderText = "60";
        var timeoutGroup = Field(T("hooks.timeout"), NativeDialogVisuals.Input(timeout, Design));
        var invalid = Hint(L("请输入正整数，或留空使用 60 秒。", "Enter a positive whole number, or leave blank for 60 seconds."));
        invalid.Visibility = model.ValidTimeout ? Visibility.Collapsed : Visibility.Visible; timeout.TextChanged += (_, _) => invalid.Visibility = model.ValidTimeout ? Visibility.Collapsed : Visibility.Visible; timeoutGroup.Children.Add(invalid);
        var hint = Hint(T(NativeHooksModel.EventHintKey(S(draft, "event"))));
        var command = Input(T("hooks.command"), S(draft, "command"), value => model.SetDraftText("command", value), true); command.PlaceholderText = "python3 .wisp/hooks/check.py";
        commandEditor = command; command.Loaded += (_, _) => ResizeCommandEditor(); ResizeCommandEditor();
        var commandGroup = Field(T("hooks.command"), NativeDialogVisuals.Input(command, Design)); commandGroup.Children.Add(Hint(T("hooks.command_hint")));
        foreach (var child in new FrameworkElement[] { eventGroup, matcherGroup, timeoutGroup, hint, commandGroup }) fields.Children.Add(child);
        (bool Tool, bool Narrow)? layoutKey = null;
        void Layout()
        {
            if (model.Draft == null || model.Closed) return;
            var tool = NativeHooksModel.MatchesTools(S(model.Draft, "event")); var narrow = NativeSettingsLayout.DefinitionFieldsStack(XamlRoot?.Size.Width ?? ActualWidth);
            hint.Text = T(NativeHooksModel.EventHintKey(S(model.Draft, "event")));
            if (layoutKey == (tool, narrow)) return; layoutKey = (tool, narrow);
            matcherGroup.Visibility = tool ? Visibility.Visible : Visibility.Collapsed;
            var hintRow = tool ? narrow ? 3 : 2 : narrow ? 2 : 1;
            fields.RowDefinitions.Clear(); for (var i = 0; i < hintRow + 2; i++) fields.RowDefinitions.Add(new() { Height = GridLength.Auto });
            Grid.SetColumnSpan(eventGroup, narrow ? 2 : 1); Grid.SetRow(matcherGroup, narrow ? 1 : 0); Grid.SetColumn(matcherGroup, narrow ? 0 : 1); Grid.SetColumnSpan(matcherGroup, narrow ? 2 : 1);
            Grid.SetRow(timeoutGroup, tool ? (narrow ? 2 : 1) : (narrow ? 1 : 0)); Grid.SetColumn(timeoutGroup, !tool && !narrow ? 1 : 0); Grid.SetColumnSpan(timeoutGroup, narrow ? 2 : 1);
            Grid.SetRow(hint, hintRow); Grid.SetColumnSpan(hint, 2); Grid.SetRow(commandGroup, hintRow + 1); Grid.SetColumnSpan(commandGroup, 2);
        }
        eventChoice.SelectionChanged += (_, _) => { if (eventChoice.SelectedItem is string value) { model.SetDraftText("event", value); Layout(); } };
        fields.SizeChanged += (_, _) => Layout(); fields.Loaded += (_, _) => Layout(); Layout(); body.Children.Add(fields);
        var actions = new NativeActionWrap { Gap = 8, HorizontalAlignment = HorizontalAlignment.Right };
        actions.Children.Add(Button(T("settings.cancel"), () => { RequestLeave(() => { model.Discard(); Commands(); }); return Task.CompletedTask; }));
        save = Button(T("settings.save"), async () => { if (await model.SaveAsync() && !model.Closed) Commands(); }, primary: true);
        save.Resources["ButtonBackgroundDisabled"] = Design.Brush("clay"); save.Resources["ButtonBorderBrushDisabled"] = Design.Brush("clay");
        if (save.Content is TextBlock caption) caption.Foreground = Design.Brush("on-clay");
        actions.Children.Add(save); foreach (var action in actions.Children.OfType<Button>()) action.MinWidth = 84;
        body.Children.Add(new Border { Child = actions, Padding = new Thickness(0, 14, 0, 0), Margin = new Thickness(0, 4, 0, 0), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 1, 0, 0) });
        commandHost.Children.Add(new Border { Child = body, Background = Design.Brush("bg-sunken"), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(1), Padding = new Thickness(16, 14, 16, 14), Margin = new Thickness(0, 4, 0, 0), CornerRadius = new CornerRadius(10) });
    }
    private StackPanel Field(string label, FrameworkElement control)
    {
        var field = new StackPanel { Spacing = 5 }; var caption = Design.Text(label, 12); caption.Foreground = Design.Brush("text-muted"); caption.FontWeight = Microsoft.UI.Text.FontWeights.Medium;
        field.Children.Add(caption); field.Children.Add(control); return field;
    }
    private TextBox Input(string label, string value, Action<string> changed, bool multiline = false)
    {
        var input = new TextBox { AcceptsReturn = multiline, Text = value, TextWrapping = TextWrapping.Wrap, CornerRadius = new CornerRadius(10), Padding = new Thickness(11, 9, 11, 9),
            Background = Design.Brush("bg-input"), BorderBrush = Design.Brush("border-strong"), BorderThickness = new Thickness(1) };
        Design.BindTypography(input, code: multiline, size: 14);
        if (multiline) { input.Padding = new Thickness(3); input.IsSpellCheckEnabled = false; input.IsTextPredictionEnabled = false; ScrollViewer.SetVerticalScrollBarVisibility(input, ScrollBarVisibility.Auto); }
        AutomationProperties.SetName(input, label); textMenus.Register(input); input.TextChanged += (_, _) => changed(input.Text); return input;
    }
    private void ResizeCommandEditor()
    {
        if (commandEditor == null || model.Closed) return;
        var lines = new TextBlock { Text = "Ag\nAg\nAg", FontFamily = Design.CodeFont, FontSize = Design.FontSize(14), TextWrapping = TextWrapping.NoWrap };
        lines.Measure(new Windows.Foundation.Size(double.PositiveInfinity, double.PositiveInfinity));
        commandEditor.Height = NativeSettingsLayout.HookCommandHeight(lines.DesiredSize.Height);
    }
    private void ProjectHooks()
    {
        projectHost.Children.Clear();
        if (model.ProjectError != null) projectHost.Children.Add(Hint(model.ProjectError));
        if (model.ProjectFile is not { } file) return;
        var trusted = file["trusted"]!.GetValue<bool>(); var valid = file["error"] == null && file["hooks"] is JsonArray { Count: > 0 };
        var heading = Group("hooks.project");
        if (trusted || valid)
        {
            var hash = trusted ? null : S(file, "sha256"); var trust = Button(T(trusted ? "hooks.project_revoke" : "hooks.project_trust"), () =>
            {
                RequestLeave(() => { model.Discard(); _ = Trust(hash); }); return Task.CompletedTask;
            }); if (!trusted) Design.ActionButton(trust, true); projectHost.Children.Add(Row(heading, trust));
        }
        else projectHost.Children.Add(heading);
        projectHost.Children.Add(Hint(Design.Localization.Format("hooks.project_desc", ("path", S(file, "path")))));
        projectHost.Children.Add(Status(T(trusted ? "hooks.project_trusted" : "hooks.project_untrusted")));
        if (file["error"] != null) projectHost.Children.Add(Hint(S(file, "error")));
        foreach (var hook in ((JsonArray)file["hooks"]!).OfType<JsonObject>()) projectHost.Children.Add(new Border { Child = HookCopy(hook, true), Padding = new Thickness(0, 10, 0, 10), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 0, 0, 1) });
        Design.ApplyTypography(projectHost);
    }
    private async Task Trust(string? hash) { await model.SetProjectTrustAsync(hash); if (!model.Closed) { Commands(); ProjectHooks(); } }
    public override void Dispose() { Design.TypographyChanged -= ResizeCommandEditor; model.Changed -= RefreshControls; textMenus.Dispose(); base.Dispose(); }
}
