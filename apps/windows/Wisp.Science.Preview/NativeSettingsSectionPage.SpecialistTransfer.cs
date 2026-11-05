using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed partial class NativeSettingsSectionPage
{
    public event Action<FrameworkElement?>? ModalChanged;
    public Func<NativeSpecialistChat, Task>? StartSpecialistChat { get; set; }
    private NativeSpecialistTransferModel? specialistTransfer;
    private object? transferIdentity;
    private Button? transferSubmit;
    private Button? transferClose;
    private CheckBox? transferReviewed;
    private TextBlock? transferError, transferNotice;
    private readonly List<Control> transferControls = [];
    private ComboBox? transferModel;

    private Button SpecialistAddMenu()
    {
        var button = Button(T("specialists.add"), () => Task.CompletedTask);
        var content = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 7 };
        content.Children.Add(Design.Text(T("specialists.add"), 13)); content.Children.Add(Design.Icon("chevron-down", 12)); button.Content = content;
        var menu = new MenuFlyout { Placement = FlyoutPlacementMode.BottomEdgeAlignedRight }; textMenus.Register(menu);
        var scratch = new MenuFlyoutItem { Text = T("specialists.add.scratch") };
        scratch.Click += (_, _) => EditSpecialist(NativeWorkflowDrafts.Specialist()); menu.Items.Add(scratch);
        var chat = new MenuFlyoutItem { Text = T("specialists.add.chat"), IsEnabled = projectScoped };
        if (!projectScoped) ToolTipService.SetToolTip(chat, L("请先打开项目。", "Open a project first."));
        chat.Click += async (_, _) =>
        {
            if (specialistTransfer == null || StartSpecialistChat == null || Busy) return;
            if (await specialistTransfer.CreateChatAsync(T("specialists.chat_prompt")) && specialistTransfer.Chat is { } launch)
            {
                // Keep navigation locked until the exact new session has been mounted.
                await specialistTransfer.RunAsync(async () => { await StartSpecialistChat(launch); return true; }, _ => { });
            }
        }; menu.Items.Add(chat);
        var import = new MenuFlyoutItem { Text = T("specialists.add.import") };
        import.Click += async (_, _) => { if (!Busy && specialistTransfer != null) await specialistTransfer.PickAsync(); }; menu.Items.Add(import);
        button.Flyout = menu; return button;
    }
    private Button SpecialistExportButton(JsonObject saved, bool iconOnly)
    {
        var button = Design.ToolButton(T("specialists.export"), "download");
        if (!iconOnly)
        {
            var content = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 7 };
            content.Children.Add(Design.Icon("download", 15)); content.Children.Add(Design.Text(T("specialists.export"), 13)); button.Content = content;
            Design.ActionButton(button);
        }
        ToolTipService.SetToolTip(button, T("specialists.export.saved_hint"));
        button.Click += async (_, _) => { if (!Busy && specialistTransfer != null) await specialistTransfer.ExportAsync(saved); };
        return button;
    }
    private void SpecialistTransferNotice(StackPanel body)
    {
        transferNotice = SpecialistHint(""); body.Children.Add(transferNotice);
        AutomationProperties.SetLiveSetting(transferNotice, Microsoft.UI.Xaml.Automation.Peers.AutomationLiveSetting.Polite);
        RefreshSpecialistTransfer();
    }
    private void RefreshSpecialistTransfer()
    {
        var state = specialistTransfer;
        if (state == null || model.Closed) return;
        var identity = (object?)state.Preview ?? state.Findings;
        if (!ReferenceEquals(identity, transferIdentity))
        {
            HideSpecialistTransfer(); transferIdentity = identity;
            if (state.Preview is { } preview) ShowSpecialistImport(preview);
            else if (state.Findings is { } findings) ShowSpecialistExport(findings);
        }
        foreach (var control in transferControls) control.IsEnabled = !state.Busy;
        if (transferReviewed != null) transferReviewed.IsEnabled = !state.Busy && !state.HasConflict;
        if (transferSubmit != null)
        {
            transferSubmit.IsEnabled = state.Preview != null ? state.CanImport : !state.Busy;
            transferSubmit.Opacity = transferSubmit.IsEnabled ? 1 : .55;
        }
        var error = state.Error == null ? "" : state.Operation == "chat" ? state.Error : Design.Localization.Format("specialists." + state.Operation + ".failed", ("msg", state.Error));
        if (transferError != null) { transferError.Text = error; transferError.Visibility = error.Length == 0 ? Visibility.Collapsed : Visibility.Visible; }
        if (transferNotice != null)
        {
            transferNotice.Text = state.ModalOpen ? "" : error.Length > 0 ? error : state.NoticeKey == null ? "" : Design.Localization.Format(state.NoticeKey, (state.Operation == "import" ? "name" : "path", state.NoticeValue));
            transferNotice.Visibility = transferNotice.Text.Length == 0 ? Visibility.Collapsed : Visibility.Visible;
        }
        LockConfirmation();
    }
    private void HideSpecialistTransfer()
    {
        ModalChanged?.Invoke(null); transferIdentity = null; transferSubmit = null; transferClose = null; transferReviewed = null; transferError = null;
        if (transferModel != null) { choices.Remove(transferModel); transferModel = null; }
        transferControls.Clear();
    }
    private Task CancelSpecialistTransfer() { specialistTransfer?.Cancel(); return Task.CompletedTask; }
    private StackPanel TransferHeader(string title)
    {
        var body = new StackPanel { Spacing = 14, Padding = new Thickness(22) };
        var heading = Design.Text(title, 17); heading.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        var close = Design.ToolButton(T("settings.cancel"), "close"); transferClose = close;
        close.Click += (_, _) => specialistTransfer?.Cancel(); transferControls.Add(close);
        body.Children.Add(QuickActionRow(heading, close, 0)); return body;
    }
    private void StyleTransferActions(Button cancel)
    {
        if (transferSubmit == null) return;
        foreach (var button in new[] { cancel, transferSubmit })
        {
            button.Padding = new Thickness(16, 9, 16, 9); button.CornerRadius = new CornerRadius(10);
            if (button.Content is TextBlock label) Design.BindTypography(label, 13.5);
        }
        cancel.Background = Design.Brush("bg-input"); cancel.BorderBrush = Design.Brush("border-strong");
        transferSubmit.Resources["ButtonBackgroundDisabled"] = Design.Brush("clay");
        transferSubmit.Resources["ButtonBorderBrushDisabled"] = Design.Brush("clay");
        if (transferSubmit.Content is TextBlock text) { text.Foreground = Design.Brush("on-clay"); text.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; }
    }
    private StackPanel TransferSection(StackPanel body, string key)
    {
        var field = new StackPanel { Spacing = 6 }; var label = Design.Text(T(key), 13); label.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        field.Children.Add(label); body.Children.Add(field); return field;
    }
    private void TransferWhitelist(StackPanel body, JsonObject package, JsonObject preview, string key)
    {
        var entries = package[key] as JsonArray;
        var missing = (preview["missing_" + key] as JsonArray ?? []).Select(n => n!.GetValue<string>()).ToHashSet(StringComparer.Ordinal);
        var field = TransferSection(body, "specialists." + key);
        if (entries == null || entries.Count == 0) field.Children.Add(SpecialistHint(T(entries == null ? "specialists.inherit" : "specialists.import.none_allowed")));
        else
        {
            var chips = new NativeActionWrap(); field.Children.Add(chips);
            foreach (var entry in entries)
            {
                var name = entry!.GetValue<string>();
                var text = Design.Text(missing.Contains(name) ? name + " · " + T("specialists.import.missing") : name, 13);
                if (missing.Contains(name)) text.Foreground = Design.Brush("text-muted");
                chips.Children.Add(new Border { Child = text, Padding = new Thickness(8, 3, 8, 3), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(4) });
            }
        }
    }
    private void ShowSpecialistImport(JsonObject preview)
    {
        var state = specialistTransfer!; var package = (JsonObject)preview["package"]!;
        var body = TransferHeader(T("specialists.import.title")); body.Children.Add(SpecialistHint(T("specialists.import.safety")));
        if (preview["name_conflict"] is { } conflict) body.Children.Add(Warn(Design.Localization.Format("specialists.import.conflict", ("name", S(conflict, "name")))));
        var fields = new Grid { ColumnSpacing = 14, RowSpacing = 6 };
        fields.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); fields.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        void Field(string key, string value)
        {
            var row = fields.RowDefinitions.Count; fields.RowDefinitions.Add(new() { Height = GridLength.Auto });
            var label = Design.Text(T(key), 13); label.Foreground = Design.Brush("text-muted"); var text = Design.Text(value, 13);
            Grid.SetRow(label, row); Grid.SetRow(text, row); Grid.SetColumn(text, 1); fields.Children.Add(label); fields.Children.Add(text);
        }
        Field("specialists.name", S(package, "name"));
        if (!string.IsNullOrWhiteSpace(S(package, "description"))) Field("specialists.description", S(package, "description"));
        var bytes = preview["bytes"]!.GetValue<long>();
        Field("specialists.import.source", S(preview, "file_name") + " · " + (bytes < 1024 ? $"{bytes} B" : $"{bytes / 1024d:0.0} KB")); body.Children.Add(fields);
        var instructions = S(package, "instructions");
        var input = new TextBox { AcceptsReturn = true, Text = string.IsNullOrWhiteSpace(instructions) ? T("specialists.import.no_instructions") : instructions,
            IsReadOnly = true, TextWrapping = TextWrapping.Wrap, MaxHeight = 280, Background = Design.Brush("bg-sunken"), CornerRadius = new CornerRadius(8), FontSize = 12 };
        Design.BindTypography(input, code: true, size: 12); AutomationProperties.SetName(input, T("specialists.instructions"));
        ScrollViewer.SetVerticalScrollBarVisibility(input, ScrollBarVisibility.Auto); textMenus.Register(input); TransferSection(body, "specialists.instructions").Children.Add(input);
        TransferWhitelist(body, package, preview, "skills");
        if (!B(preview, "skills_checked") && package["skills"] is JsonArray { Count: > 0 }) body.Children.Add(SpecialistHint(T("specialists.import.skills_unchecked")));
        TransferWhitelist(body, package, preview, "connectors");
        if (preview["missing_skills"] is JsonArray { Count: > 0 } || preview["missing_connectors"] is JsonArray { Count: > 0 }) body.Children.Add(SpecialistHint(T("specialists.import.missing_hint")));
        var modelField = TransferSection(body, "specialists.model");
        if (package["model_hint"] is { } hint) modelField.Children.Add(SpecialistHint(Design.Localization.Format("specialists.import.model_hint", ("provider", S(hint, "provider")), ("model", S(hint, "model")))));
        if (state.Matches.Any())
        {
            transferModel = new ComboBox { HorizontalAlignment = HorizontalAlignment.Stretch };
            AutomationProperties.SetName(transferModel, T("specialists.model"));
            transferModel.Items.Add(new ComboBoxItem { Content = T("specialists.model.follow"), Tag = "" });
            foreach (var match in state.Matches) transferModel.Items.Add(new ComboBoxItem { Content = S(match, "label") + " · " + S(match, "provider"), Tag = S(match, "id") });
            transferModel.SelectedItem = transferModel.Items.Cast<ComboBoxItem>().First(item => (string)item.Tag == state.ModelId);
            transferModel.SelectionChanged += (_, _) => { if (transferModel?.SelectedItem is ComboBoxItem item) state.SetModel((string)item.Tag); };
            choices.Add(transferModel); transferControls.Add(transferModel); modelField.Children.Add(transferModel);
        }
        else modelField.Children.Add(SpecialistHint(T("specialists.import.model_follow")));
        transferReviewed = new CheckBox { Content = Design.Text(T("specialists.import.reviewed"), 13), IsChecked = state.Reviewed };
        transferReviewed.Checked += (_, _) => state.SetReviewed(true); transferReviewed.Unchecked += (_, _) => state.SetReviewed(false); body.Children.Add(transferReviewed);
        transferError = Warn(""); body.Children.Add(transferError);
        var actions = new NativeActionWrap { RightAligned = true }; var cancel = Button(T("settings.cancel"), CancelSpecialistTransfer); actions.Children.Add(cancel); transferControls.Add(cancel);
        transferSubmit = Button(T("specialists.import.confirm"), async () =>
        {
            if (await state.ImportAsync() && state.Imported is { } imported && !model.Closed)
            { model.Values["list_specialists"] = imported.DeepClone(); Render(); }
        }); Design.ActionButton(transferSubmit, true); StyleTransferActions(cancel); actions.Children.Add(transferSubmit); body.Children.Add(actions);
        ShowSpecialistTransfer(body, T("specialists.import.title"));
    }
    private void ShowSpecialistExport(JsonArray findings)
    {
        var state = specialistTransfer!; var body = TransferHeader(T("specialists.export.secret_title"));
        body.Children.Add(SpecialistHint(Design.Localization.Format("specialists.export.secret_body", ("name", state.ExportName))));
        foreach (var finding in findings)
        {
            var kind = S(finding, "kind"); var label = T("specialists.secret." + kind);
            var field = new StackPanel { Spacing = 6 };
            field.Children.Add(Design.Text(Design.Localization.Format("specialists.export.secret_line", ("line", finding!["line"]!.ToString()), ("kind", label == "specialists.secret." + kind ? kind : label)), 13));
            var excerpt = new TextBox { IsReadOnly = true, AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, Text = S(finding, "excerpt") };
            Design.BindTypography(excerpt, code: true, size: 12); textMenus.Register(excerpt); field.Children.Add(excerpt); body.Children.Add(field);
        }
        transferError = Warn(""); body.Children.Add(transferError);
        var actions = new NativeActionWrap { RightAligned = true }; var cancel = Button(T("settings.cancel"), CancelSpecialistTransfer); actions.Children.Add(cancel); transferControls.Add(cancel);
        transferSubmit = Button(T("specialists.export.anyway"), async () => { await state.ExportAnywayAsync(); }); Design.ActionButton(transferSubmit, true); StyleTransferActions(cancel); actions.Children.Add(transferSubmit); body.Children.Add(actions);
        ShowSpecialistTransfer(body, T("specialists.export.secret_title"));
    }
    private void ShowSpecialistTransfer(StackPanel body, string name)
    {
        var overlay = new Grid(); NativeDialogVisuals.Backdrop(overlay, Design); AutomationProperties.SetName(overlay, name);
        var card = new Border { Child = new ScrollViewer { Content = body, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch },
            Background = Design.Brush("bg-elev"), BorderBrush = Design.Brush("border-strong"), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(16), HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center, TabFocusNavigation = KeyboardNavigationMode.Cycle,
            Shadow = new ThemeShadow(), Translation = new System.Numerics.Vector3(0, 0, 24) };
        overlay.Children.Add(card);
        void Resize() { card.Width = Math.Min(640, Math.Max(0, overlay.ActualWidth - 40)); card.MaxHeight = Math.Max(0, overlay.ActualHeight - 40); body.Width = Math.Max(0, card.Width - 2); }
        overlay.SizeChanged += (_, _) => Resize();
        // The footer can be outside the narrow viewport, and conflict confirmation
        // is disabled. Restore keyboard focus to the visible header after the host
        // picker closes; Escape still belongs to the window's shared surface stack.
        overlay.Loaded += (_, _) => { Resize(); RestoreSpecialistTransferFocus(); };
        overlay.Tapped += (_, e) => { if (ReferenceEquals(e.OriginalSource, overlay)) specialistTransfer?.Cancel(); };
        ModalChanged?.Invoke(overlay); Design.ApplyTypography(body);
    }
    private void RestoreSpecialistTransferFocus()
    {
        var target = transferClose;
        if (target == null || specialistTransfer?.ModalOpen != true) return;
        DispatcherQueue.TryEnqueue(Microsoft.UI.Dispatching.DispatcherQueuePriority.Low, () =>
        {
            if (ReferenceEquals(target, transferClose) && specialistTransfer?.ModalOpen == true && target.IsLoaded && target.IsEnabled)
            {
                target.Focus(FocusState.Programmatic);
            }
        });
    }
}
