using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed class NativeProjectSettingsPage : NativeActionPage
{
    protected override ProjectDialogLayout ModalBounds(double width, double height, double preferredWidth)
        => ProjectDialogLayout.ForProjectForm(width, height);
    private readonly NativeProjectSettingsModel model;
    private readonly Func<Task> saved, folderChanged;
    private readonly Action<FlyoutBase> register;
    private readonly StackPanel actions = new() { Orientation = Orientation.Horizontal, Spacing = 10, HorizontalAlignment = HorizontalAlignment.Right };
    private readonly TextBlock retentionError;
    private readonly ContentControl modalHost = new() { IsTabStop = false, HorizontalContentAlignment = HorizontalAlignment.Stretch, VerticalContentAlignment = VerticalAlignment.Stretch };
    private Button? save, cancel, enableSync;
    private TextBlock? syncEnabled, syncError;
    private TextBox? name;
    private readonly List<TextBox> retentionInputs = [];
    private Grid? confirmation;
    private bool started, mounted;

    public NativeProjectSettingsPage(NativeProjectSettingsModel model, WispDesign design, Func<Task> saved,
        Func<Task> folderChanged, Action close, Action<FlyoutBase> register)
        : base(design, design.T("proj_settings.title"), model, close, modal: true, scrollWholeModal: true)
    {
        this.model = model; this.saved = saved; this.folderChanged = folderChanged; this.register = register;
        UseLayoutRounding = false;
        var layers = (Grid)Content; var surface = layers.Children.Last(); layers.Children.Remove(surface); modalHost.Content = surface; layers.Children.Add(modalHost);
        Form.Spacing = 14; Form.MaxWidth = double.PositiveInfinity;
        AutomationProperties.SetName(this, design.T("proj_settings.title"));
        AutomationProperties.SetAutomationId(this, "project-settings-dialog");
        retentionError = Hint(design.L("请填写 1–3650 的整数，或留空关闭自动清理。", "Enter a whole number from 1 to 3650, or leave blank to turn cleanup off."));
        retentionError.Foreground = design.Brush("err"); retentionError.Visibility = Visibility.Collapsed;
        Form.Children.Add(Hint(design.L("正在读取项目设置…", "Loading project settings…")));
        model.Changed += Refresh;
        Loaded += async (_, _) => { if (!started) { started = true; await model.LoadAsync(); } };
    }
    private void MountForm()
    {
        mounted = true; Form.Children.Clear();
        name = Editor("name", "", model.Name, value => { model.Name = value; Refresh(); });
        Editor("description", "description_hint", model.Description, value => model.Description = value, 2);
        Editor("agent_context", "agent_context_hint", model.AgentContext, value => model.AgentContext = value, 8);
        if (model.InWorkspace)
        {
            var group = Group("retention", "retention_hint");
            var rows = new StackPanel { Spacing = 6 }; group.Children.Add(rows);
            foreach (var key in new[] { "retention_succeeded", "retention_failed", "retention_orphan" })
            {
                var row = new Grid { ColumnSpacing = 12 };
                row.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); row.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
                var label = Hint(Design.T("proj_settings." + key)); Design.BindTypography(label, 12.5); label.Foreground = Design.Brush("text-muted"); label.VerticalAlignment = VerticalAlignment.Center; row.Children.Add(label);
                var field = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 };
                var input = new TextBox { Width = 84, MinHeight = 32, Padding = new Thickness(9, 6, 9, 6), TextAlignment = TextAlignment.Right,
                    CornerRadius = new CornerRadius(10), Background = Design.Brush("bg-input"), Foreground = Design.Brush("text"), BorderBrush = Design.Brush("border-strong"), BorderThickness = new Thickness(1) };
                var menu = new TextCommandBarFlyout(); input.ContextFlyout = menu; register(menu);
                Design.BindTypography(input, size: 13); input.PlaceholderText = Design.T("proj_settings.retention_off");
                AutomationProperties.SetName(input, label.Text); retentionInputs.Add(input); field.Children.Add(NativeDialogVisuals.Input(input, Design));
                var unit = Hint(Design.T("proj_settings.retention_days_unit")); Design.BindTypography(unit, 12); unit.MinWidth = 24; unit.VerticalAlignment = VerticalAlignment.Center; field.Children.Add(unit);
                async Task Commit() => await model.SaveRetentionAsync(retentionInputs.Select(box => box.Text).ToArray());
                PreferenceNumberBox.Apply(input, Design, 1, 3650, Commit);
                input.LostFocus += async (_, _) => await Commit();
                input.PreviewKeyDown += async (_, e) => { if (e.Key == Windows.System.VirtualKey.Enter) { e.Handled = true; await Commit(); } };
                Grid.SetColumn(field, 1); row.Children.Add(field); rows.Children.Add(row);
            }
            group.Children.Add(retentionError);
            var retry = Action(Design.L("重新读取清理设置", "Reload cleanup settings"), async () => await model.LoadRetentionAsync(reconnectFirst: true));
            model.Changed += () => { retry.Visibility = model.RetentionReady ? Visibility.Collapsed : Visibility.Visible; };
            group.Children.Add(retry);
        }
        else
        {
            var group = Group("folder_sync", "folder_sync_hint"); group.Spacing = 6;
            syncEnabled = Hint(Design.T("proj_settings.folder_sync_enabled")); group.Children.Add(syncEnabled);
            enableSync = Action(Design.T("proj_settings.folder_sync_enable"), async () => { if (await model.EnableFolderSyncAsync()) await folderChanged(); }); group.Children.Add(enableSync);
            syncError = Design.Text("", 12); syncError.Foreground = Design.Brush("err");
            AutomationProperties.SetAutomationId(syncError, "project-folder-sync-error");
            AutomationProperties.SetLiveSetting(syncError, Microsoft.UI.Xaml.Automation.Peers.AutomationLiveSetting.Assertive);
            group.Children.Add(syncError);
        }
        cancel = Action(Design.T("settings.cancel"), () => { HandleEscape(); return Task.CompletedTask; });
        save = Action(Design.T("settings.save"), async () => { if (await model.SaveAsync()) await saved(); }, primary: true);
        actions.Children.Add(cancel); actions.Children.Add(save); Footer.Children.Add(actions); Footer.Visibility = Visibility.Visible;
        name.Loaded += (_, _) => FocusName();
    }
    private void FocusName() { if (name?.Focus(FocusState.Programmatic) == true) name.SelectAll(); }
    private void Refresh()
    {
        if (model.Closed) return;
        if (model.Loaded && !mounted) MountForm();
        if (!model.Loaded && !model.Busy)
        {
            Form.Children.Clear(); Form.Children.Add(Action(Design.L("重新连接并读取", "Reconnect and reload"), async () => await model.LoadAsync(reconnectFirst: true)));
        }
        if (save != null) { save.IsEnabled = model.CanSave; save.Opacity = save.IsEnabled ? 1 : .55; }
        if (cancel != null) cancel.IsEnabled = !model.Busy && !model.EnablingFolderSync;
        if (enableSync != null) { enableSync.Visibility = model.FolderSync ? Visibility.Collapsed : Visibility.Visible; enableSync.IsEnabled = !model.Busy && !model.EnablingFolderSync; }
        if (syncEnabled != null) syncEnabled.Visibility = model.FolderSync ? Visibility.Visible : Visibility.Collapsed;
        if (syncError != null)
        {
            syncError.Text = model.FolderSyncError ?? "";
            syncError.Visibility = model.FolderSyncError == null ? Visibility.Collapsed : Visibility.Visible;
        }
        if (retentionInputs.Count > 0)
        {
            for (var i = 0; i < retentionInputs.Count; i++)
            {
                var input = retentionInputs[i]; input.IsEnabled = model.RetentionReady && !model.Busy;
                input.IsReadOnly = model.RetentionSaving;
                if (model.RetentionReady && input.Tag == null) { input.Text = model.Retention[i]; input.Tag = "loaded"; }
            }
            retentionError.Visibility = model.RetentionInvalid ? Visibility.Visible : Visibility.Collapsed;
        }
        if (model.ConfirmationOpen && confirmation == null) ShowConfirmation();
        else if (!model.ConfirmationOpen && confirmation != null)
        {
            ((Grid)Content).Children.Remove(confirmation); confirmation = null; modalHost.IsEnabled = true;
            if (!model.Busy) { if (model.InWorkspace) save?.Focus(FocusState.Programmatic); else FocusName(); }
        }
    }
    private StackPanel Group(string key, string hint = "")
    {
        var group = new StackPanel { Spacing = 5 };
        var folderHeading = key == "folder_sync";
        var label = Design.Text(Design.T("proj_settings." + key), folderHeading ? 14 : 12);
        label.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; label.Foreground = Design.Brush(folderHeading ? "text" : "text-muted"); group.Children.Add(label);
        if (hint.Length > 0) group.Children.Add(Hint(Design.T("proj_settings." + hint)));
        Form.Children.Add(group); return group;
    }
    private TextBlock Hint(string text)
    {
        var block = Design.Text(text, 11.5); Design.BindLineHeight(block, 11.5, 1.4);
        block.LineStackingStrategy = LineStackingStrategy.BlockLineHeight;
        block.Foreground = Design.Brush("text-faint"); return block;
    }
    private TextBox Editor(string key, string hint, string value, Action<string> changed, int rows = 0)
    {
        var input = ProjectEditor(Group(key, hint), value, changed, rows, register);
        AutomationProperties.SetAutomationId(input, "project-settings-" + key); return input;
    }
    private Button Action(string label, Func<Task> action, bool primary = false)
    {
        var button = Button(label, action, primary); var caption = Design.Text(label, 13.5); button.Content = caption; button.Padding = new Thickness(16, 9, 16, 9); button.CornerRadius = new CornerRadius(10);
        if (primary) { caption.Foreground = Design.Brush("on-clay"); caption.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; }
        if (primary) { button.Resources["ButtonBackgroundDisabled"] = Design.Brush("clay"); button.Resources["ButtonBorderBrushDisabled"] = Design.Brush("clay"); }
        else { button.Background = Design.Brush("bg-input"); button.BorderBrush = Design.Brush("border-strong"); }
        return button;
    }
    private void ShowConfirmation()
    {
        var layers = (Grid)Content; modalHost.IsEnabled = false;
        confirmation = new Grid(); NativeDialogVisuals.Backdrop(confirmation, Design);
        var content = new Grid { RowSpacing = 14, Padding = new Thickness(22) };
        content.RowDefinitions.Add(new() { Height = GridLength.Auto });
        content.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) });
        content.RowDefinitions.Add(new() { Height = GridLength.Auto });
        var title = Design.Text(Design.T("proj_settings.agent_context_confirm_action"), 17); title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; content.Children.Add(title);
        var message = Design.Text(Design.T("proj_settings.agent_context_confirm"), 13);
        Design.BindLineHeight(message, 13, 1.55); message.Foreground = Design.Brush("text-muted");
        var explanation = new ScrollViewer { Content = message, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled,
            HorizontalContentAlignment = HorizontalAlignment.Stretch, VerticalScrollBarVisibility = ScrollBarVisibility.Auto };
        Grid.SetRow(explanation, 1); content.Children.Add(explanation);
        var buttons = new NativeActionWrap { HorizontalAlignment = HorizontalAlignment.Stretch, RightAligned = true, Gap = 10, Margin = new Thickness(0, 4, 0, 0) };
        var no = Action(Design.T("settings.cancel"), () => { model.CancelConfirmation(); return Task.CompletedTask; }); buttons.Children.Add(no);
        buttons.Children.Add(Action(Design.T("proj_settings.agent_context_confirm_action"), async () => { if (await model.SaveAsync(confirmContext: true)) await saved(); }, primary: true)); Grid.SetRow(buttons, 2); content.Children.Add(buttons);
        var card = new Border { Child = content,
            Shadow = new ThemeShadow(), Translation = new System.Numerics.Vector3(0, 0, 24),
            Background = Design.Brush("bg-elev"), BorderBrush = Design.Brush("border-strong"), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(16), HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center, TabFocusNavigation = KeyboardNavigationMode.Cycle };
        void Resize() { var bounds = ProjectDialogLayout.ForViewport(ActualWidth, ActualHeight, 480); card.Width = bounds.Width; card.MaxHeight = bounds.MaxHeight; }
        confirmation.SizeChanged += (_, _) => Resize(); confirmation.Loaded += (_, _) => { Resize(); no.Focus(FocusState.Programmatic); };
        confirmation.Children.Add(card); layers.Children.Add(confirmation);
    }
    public override void HandleEscape() { if (model.ConfirmationOpen) { model.CancelConfirmation(); return; } base.HandleEscape(); }
    public override void Dispose() { model.Changed -= Refresh; base.Dispose(); }
}
