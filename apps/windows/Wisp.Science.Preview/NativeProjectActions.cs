using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

/// <summary>Shared lifecycle for project action forms; inputs remain mounted across failed operations.</summary>
internal abstract class NativeActionPage : WorkspaceSheet
{
    protected readonly WorkspaceActionModel State;
    protected readonly StackPanel Form = new() { Spacing = 12, MaxWidth = 900, HorizontalAlignment = HorizontalAlignment.Stretch };
    protected readonly StackPanel Results = new() { Spacing = 12 };
    private readonly TextBlock error = new() { TextWrapping = TextWrapping.Wrap };
    private readonly ProgressBar progress = new() { IsIndeterminate = true, Height = 3 };
    private readonly ContentControl formHost = new();
    private readonly ContentControl resultsHost = new();
    private readonly bool ownsState;
    protected virtual bool ErrorHandledInline => false;
    protected NativeActionPage(WispDesign design, string title, WorkspaceActionModel state, Action close, bool ownsState = true, bool modal = false, double modalWidth = ProjectDialogLayout.PreferredWidth,
        bool scrollWholeModal = false) : base(design, title, close, modal, modalWidth, scrollWholeModal)
    {
        State = state; this.ownsState = ownsState;
        formHost.Content = Form; resultsHost.Content = Results;
        formHost.HorizontalContentAlignment = HorizontalAlignment.Stretch; resultsHost.HorizontalContentAlignment = HorizontalAlignment.Stretch;
        formHost.HorizontalAlignment = resultsHost.HorizontalAlignment = HorizontalAlignment.Stretch;
        if (modal) resultsHost.Visibility = Visibility.Collapsed;
        Body.Children.Add(progress); Body.Children.Add(error); Body.Children.Add(formHost); Body.Children.Add(resultsHost);
        state.Changed += Update; Update();
    }
    protected void Update()
    {
        if (State.Closed) return;
        Design.ApplyTypography(Form); Design.ApplyTypography(Results);
        formHost.IsEnabled = !State.Busy; resultsHost.IsEnabled = !State.Busy;
        progress.Visibility = State.Busy ? Visibility.Visible : Visibility.Collapsed;
        foreach (var action in HeaderActions.Children.OfType<Control>()) action.IsEnabled = !State.Busy;
        error.Text = State.Error ?? ""; error.Visibility = State.Error == null || ErrorHandledInline ? Visibility.Collapsed : Visibility.Visible;
    }
    // The selected sidebar item is another way to follow the current page's
    // back button. Its own callback owns draft handling and the list position.
    public void ReselectSettingsCategory()
    {
        if (!State.Closed && !State.Busy && formHost.IsEnabled) BackToSettingsList?.Invoke();
    }
    protected void SetContentEnabled(bool enabled, bool resultsEnabled = true)
    {
        formHost.IsEnabled = enabled;
        resultsHost.IsEnabled = enabled && resultsEnabled;
        foreach (var action in HeaderActions.Children.OfType<Control>()) action.IsEnabled = enabled;
    }
    protected void HideResults() => resultsHost.Visibility = Visibility.Collapsed;
    protected TextBox ProjectEditor(StackPanel group, string value, Action<string> changed, int rows, Action<FlyoutBase> register)
    {
        var label = (TextBlock)group.Children[0];
        var multiline = rows > 0;
        var input = new TextBox { AcceptsReturn = multiline, Text = value,
            TextWrapping = multiline ? TextWrapping.Wrap : TextWrapping.NoWrap,
            Height = multiline ? ProjectDialogLayout.EditorHeight(rows, Design.FontSize(12)) : double.NaN,
            MinHeight = multiline ? ProjectDialogLayout.EditorMinimum(rows) : 36,
            Padding = new Thickness(11, multiline ? 9 : 8, 11, multiline ? 9 : 8), CornerRadius = new CornerRadius(10),
            Background = Design.Brush("bg-input"), Foreground = Design.Brush("text"),
            BorderBrush = Design.Brush("border-strong"), BorderThickness = new Thickness(1),
            HorizontalAlignment = HorizontalAlignment.Stretch };
        Design.BindTypography(input, code: multiline, size: multiline ? 12 : 14);
        if (!multiline)
        {
            input.FontWeight = Microsoft.UI.Text.FontWeights.Medium;
            PreferenceFieldAppearance.UsePlainTextField(input);
        }
        ScrollViewer.SetHorizontalScrollBarVisibility(input, ScrollBarVisibility.Disabled);
        ScrollViewer.SetVerticalScrollBarVisibility(input, multiline ? ScrollBarVisibility.Auto : ScrollBarVisibility.Disabled);
        var textMenu = new TextCommandBarFlyout(); input.ContextFlyout = textMenu; register(textMenu);
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetLabeledBy(input, label);
        // WebView wraps the caption and its help text in the editor's label.
        // Only the text forwards the tap: the editor and its resize grip keep
        // their own selection, click and drag handling.
        foreach (var caption in group.Children.OfType<TextBlock>())
        {
            if (caption != label) Microsoft.UI.Xaml.Automation.AutomationProperties.SetHelpText(input, caption.Text);
            caption.Tapped += (_, _) => { if (input.IsEnabled) input.Focus(FocusState.Pointer); };
        }
        input.TextChanged += (_, _) => changed(multiline ? input.Text.ReplaceLineEndings("\n") : input.Text);
        var ring = NativeDialogVisuals.Input(input, Design);
        if (!multiline) group.Children.Add(ring);
        else
        {
            var box = new Grid();
            box.Children.Add(ring);
            box.Children.Add(new NativeProjectTextResizeGrip(input, Design, label.Text));
            group.Children.Add(box);
        }
        return input;
    }
    protected TextBox Field(string title, string text, Action<string> changed, bool multiline = false)
    {
        // WinUI truncates initial text at the first newline unless multiline mode is set first.
        var input = new TextBox { Header = title, AcceptsReturn = multiline, Text = text, TextWrapping = TextWrapping.Wrap, MinHeight = multiline ? 85 : 32 };
        Design.BindTypography(input);
        input.TextChanged += (_, _) => changed(input.Text); Form.Children.Add(input); return input;
    }
    protected Button Button(string title, Func<Task> action, bool? primary = null)
    {
        var button = new Button { Content = title };
        Design.ActionButton(button, primary ?? (title is "保存" or "创建项目" or "导入" or "创建论文证据"));
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(button, title);
        button.Click += async (_, _) => await action(); return button;
    }
    protected Expander Disclosure(string title, UIElement content, bool expanded = false)
    {
        var expander = new Expander { Header = Design.Text(title), Content = content,
            IsExpanded = expanded, HorizontalAlignment = HorizontalAlignment.Stretch,
            HorizontalContentAlignment = HorizontalAlignment.Stretch };
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(expander, title);
        Design.ApplyTypography(expander);
        return expander;
    }
    public override void HandleEscape()
    {
        foreach (var combo in Descendants(Form).OfType<ComboBox>())
            if (combo.IsDropDownOpen) { combo.IsDropDownOpen = false; return; }
        foreach (var picker in Descendants(Form).OfType<CalendarDatePicker>())
            if (picker.IsCalendarOpen) { picker.IsCalendarOpen = false; return; }
        if (!State.Busy) base.HandleEscape();
    }
    private static IEnumerable<UIElement> Descendants(Panel panel)
    {
        foreach (var child in panel.Children)
        { yield return child; if (child is Panel nested) foreach (var descendant in Descendants(nested)) yield return descendant; }
    }
    public override void Dispose() { State.Changed -= Update; if (ownsState) State.Dispose(); base.Dispose(); }
}

internal sealed class NativeNewProjectPage : NativeActionPage
{
    protected override ProjectDialogLayout ModalBounds(double width, double height, double preferredWidth)
        => ProjectDialogLayout.ForProjectForm(width, height);
    private readonly Action updateSubmit;
    private readonly Action<FlyoutBase> register;
    public NativeNewProjectPage(WorkspaceProjectCreation model, WispDesign design, Func<Task<string?>> pickDirectory,
        Func<ProjectSummary, Task> opened, Action close, Action<FlyoutBase> register) : base(design, design.T("projects.new"), model, close, modal: true, scrollWholeModal: true)
    {
        this.register = register;
        // Let fractional text line boxes and gaps accumulate as in the CSS
        // form, instead of rounding up every row at fractional display scales.
        UseLayoutRounding = false;
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(this, design.T("projects.new"));
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(this, "new-project-dialog");
        Form.Spacing = 14;
        var picking = false;
        var create = Button(design.T("projects.create"), async () =>
        {
            if (model.CanCreate && await model.CreateAsync() && model.Created is { } row) await opened(row);
        }, primary: true);
        // ActionButton's custom text must retain the primary button foreground.
        if (create.Content is TextBlock createLabel)
        {
            createLabel.Foreground = design.Brush("on-clay");
            createLabel.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        }
        // WinUI's default disabled background is nearly white. Retain the accent
        // and dim the whole button so its white label remains readable.
        create.Resources["ButtonBackgroundDisabled"] = design.Brush("clay");
        create.Resources["ButtonBorderBrushDisabled"] = design.Brush("clay");
        var cancel = Button(design.T("projects.cancel"), () => { HandleEscape(); return Task.CompletedTask; });
        foreach (var button in new[] { cancel, create })
        {
            button.Padding = new Thickness(16, 9, 16, 9); button.CornerRadius = new CornerRadius(10);
            if (button.Content is TextBlock text) design.BindTypography(text, 13.5);
        }
        cancel.Background = design.Brush("bg-input"); cancel.BorderBrush = design.Brush("border-strong");
        cancel.Resources["ButtonBorderBrushPointerOver"] = design.Brush("text-faint");
        updateSubmit = () =>
        {
            create.IsEnabled = model.CanCreate && !picking;
            create.Opacity = create.IsEnabled ? 1 : .55;
            cancel.IsEnabled = !model.Busy && !picking;
        };
        var name = Editor(design.T("proj_settings.name"), "", model.Name, s => { model.Name = s; updateSubmit(); });
        name.PlaceholderText = design.T("projects.name_ph");
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(name, "new-project-name");

        var directoryGroup = Group(design.T("projects.directory"));
        var directoryRow = new Grid { ColumnSpacing = 8 };
        directoryRow.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        directoryRow.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        var directory = design.Text(model.Directory, 13);
        directory.TextWrapping = TextWrapping.NoWrap; directory.TextTrimming = TextTrimming.CharacterEllipsis;
        directory.Foreground = design.Brush("text-faint"); directory.VerticalAlignment = VerticalAlignment.Center;
        if (model.Directory.Length > 0) ToolTipService.SetToolTip(directory, model.Directory);
        var choose = Button(design.T("projects.choose_dir"), async () =>
        {
            if (picking || model.Busy || model.Closed) return;
            picking = true; updateSubmit(); SetContentEnabled(false);
            try
            {
                if (await pickDirectory() is { } path && !model.Closed)
                {
                    model.Directory = path; directory.Text = path;
                    ToolTipService.SetToolTip(directory, path);
                }
            }
            catch (Exception ex) { if (!model.Closed) model.Fail(design.L("无法选择项目目录：", "Could not choose the project directory: ") + ex.Message); }
            finally { picking = false; if (!model.Closed) { Update(); updateSubmit(); } }
        });
        choose.MinHeight = 0; choose.Padding = new Thickness(12, 8, 12, 8); choose.CornerRadius = new CornerRadius(10);
        if (choose.Content is TextBlock chooseLabel)
        {
            design.BindTypography(chooseLabel, 13.5);
            chooseLabel.FontWeight = Microsoft.UI.Text.FontWeights.Medium;
        }
        directoryRow.Children.Add(choose); Grid.SetColumn(directory, 1); directoryRow.Children.Add(directory);
        directoryGroup.Children.Add(directoryRow);

        Editor(design.T("proj_settings.description"), design.T("proj_settings.description_hint"),
            model.Description, s => model.Description = s, rows: 2);
        var layoutRow = new Grid { ColumnSpacing = 10 };
        layoutRow.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        layoutRow.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        var standard = new ToggleSwitch { IsOn = model.StandardLayout, OnContent = "", OffContent = "",
            MinWidth = 0, VerticalAlignment = VerticalAlignment.Top, Margin = new Thickness(0, 1, 0, 0) };
        PreferenceToggleSwitch.Apply(standard, design);
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(standard, design.T("projects.standard_layout"));
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetHelpText(standard, design.T("projects.standard_layout_hint"));
        var layoutText = new StackPanel { Spacing = 3 };
        layoutText.Children.Add(Label(design.T("projects.standard_layout")));
        layoutText.Children.Add(Hint(design.T("projects.standard_layout_hint")));
        layoutText.Tapped += (_, _) =>
        {
            if (!standard.IsEnabled) return;
            standard.Focus(FocusState.Pointer); standard.IsOn = !standard.IsOn;
        };
        layoutRow.Children.Add(standard); Grid.SetColumn(layoutText, 1); layoutRow.Children.Add(layoutText);
        Form.Children.Add(layoutRow);
        var context = Editor(design.T("proj_settings.agent_context"), design.T("proj_settings.agent_context_hint"),
            model.AgentContext, s => model.AgentContext = s, rows: 8);
        standard.Toggled += (_, _) => { model.SetStandardLayout(standard.IsOn); context.Text = model.AgentContext; };

        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 10, HorizontalAlignment = HorizontalAlignment.Right };
        actions.Children.Add(cancel); actions.Children.Add(create);
        Footer.Children.Add(actions); Footer.Visibility = Visibility.Visible;
        model.Changed += updateSubmit; updateSubmit();
        Loaded += (_, _) => { if (!model.Closed) { name.Focus(FocusState.Programmatic); name.SelectAll(); } };
    }

    private TextBlock Label(string title)
    {
        var label = Design.Text(title, 12); label.Foreground = Design.Brush("text-muted");
        label.FontWeight = Microsoft.UI.Text.FontWeights.Medium; return label;
    }
    private TextBlock Hint(string text)
    {
        var hint = Design.Text(text, 11.5); Design.BindLineHeight(hint, 11.5, 1.4);
        hint.LineStackingStrategy = LineStackingStrategy.BlockLineHeight;
        hint.Foreground = Design.Brush("text-faint"); return hint;
    }
    private StackPanel Group(string title, string hint = "")
    {
        var group = new StackPanel { Spacing = 5, HorizontalAlignment = HorizontalAlignment.Stretch };
        group.Children.Add(Label(title));
        if (hint.Length > 0) group.Children.Add(Hint(hint));
        Form.Children.Add(group); return group;
    }
    private TextBox Editor(string title, string hint, string value, Action<string> changed, int rows = 0)
        => ProjectEditor(Group(title, hint), value, changed, rows, register);
    public override void Dispose() { State.Changed -= updateSubmit; base.Dispose(); }
}

internal sealed class NativeSessionGroupPage : WorkspaceSheet
{
    private readonly WorkspaceSessionGroups groups;
    private readonly Action updateSubmit;
    public NativeSessionGroupPage(WorkspaceSessionGroups groups, WispDesign design, Action close, Action<FlyoutBase> register)
        : base(design, design.T(groups.RenamingId == null ? "folder.new_title" : "folder.rename_prompt"), close, modal: true, modalWidth: 480)
    {
        this.groups = groups; HideDismissButton();
        var field = new StackPanel { Spacing = 6 }; field.Children.Add(design.Text(design.T("folder.new_prompt"), 12));
        var name = new TextBox { Text = groups.Draft, MinHeight = 36, Padding = new(11, 8, 11, 8), CornerRadius = new(10) };
        design.BindTypography(name, size: 14); PreferenceFieldAppearance.Apply(name, design);
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(name, design.T("folder.new_prompt"));
        field.Children.Add(name); Body.Children.Add(field);
        var context = new TextCommandBarFlyout(); name.ContextFlyout = context; register(context);
        var error = design.Text("", 13); error.Foreground = design.Brush("err"); Body.Children.Add(error);
        var save = new Button { Content = design.Text(design.T("settings.save"), 14) }; design.ActionButton(save, true);
        ((TextBlock)save.Content).Foreground = design.Brush("on-clay");
        save.Resources["ButtonBackgroundDisabled"] = design.Brush("clay");
        save.Resources["ButtonBorderBrushDisabled"] = design.Brush("clay");
        async Task Save() { if (groups.CanSave && await groups.SaveAsync()) close(); }
        save.Click += async (_, _) => await Save(); name.KeyDown += async (_, e) => { if (e.Key == Windows.System.VirtualKey.Enter) { e.Handled = true; await Save(); } };
        var cancel = new Button { Content = design.Text(design.T("settings.cancel"), 14) }; design.ActionButton(cancel, false);
        cancel.Click += (_, _) => HandleEscape();
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, HorizontalAlignment = HorizontalAlignment.Right };
        actions.Children.Add(cancel); actions.Children.Add(save); Footer.Children.Add(actions); Footer.Visibility = Visibility.Visible;
        updateSubmit = () =>
        {
            save.IsEnabled = groups.CanSave; save.Opacity = groups.CanSave ? 1 : .5; name.IsReadOnly = groups.Busy; cancel.IsEnabled = !groups.Busy;
            error.Text = (groups.FolderSaveUnconfirmed ? design.L("结果未能确认，不会自动重试。关闭后请先检查分组列表。", "The result could not be confirmed and will not be retried. Close and check the group list first.") + "\n" : "") + groups.FolderFailureDetail;
            error.Visibility = groups.Error == null ? Visibility.Collapsed : Visibility.Visible;
        };
        groups.Changed += updateSubmit; name.TextChanged += (_, _) => { groups.Draft = name.Text; updateSubmit(); }; updateSubmit();
        Loaded += (_, _) => { name.Focus(FocusState.Programmatic); if (groups.RenamingId != null) name.SelectAll(); };
    }
    public override void HandleEscape() { if (!groups.Busy) base.HandleEscape(); }
    public override void Dispose() { groups.Changed -= updateSubmit; base.Dispose(); }
}

internal sealed class NativeFolderDeletePage : WorkspaceSheet
{
    private readonly WorkspaceSessionGroups groups;
    private readonly Action refresh;
    public NativeFolderDeletePage(WorkspaceSessionGroups groups, string folderId, WispDesign design, Action deleted, Action close)
        : base(design, design.T("confirm.title"), close, modal: true, modalWidth: 480)
    {
        this.groups = groups; HideDismissButton();
        var hint = design.Text(design.T("folder.delete_confirm"), 13); hint.Foreground = design.Brush("text-muted"); Body.Children.Add(hint);
        var error = design.Text("", 13); error.Foreground = design.Brush("err"); Body.Children.Add(error);
        var actions = new StackPanel { Orientation = Orientation.Horizontal, HorizontalAlignment = HorizontalAlignment.Right, Spacing = 8 };
        var cancel = new Button { Content = design.Text(design.T("settings.cancel"), 14) }; design.ActionButton(cancel, false); cancel.Click += (_, _) => HandleEscape();
        var remove = new Button { Content = design.Text(design.T("ctx.delete_folder"), 14) }; design.ActionButton(remove, true);
        ((TextBlock)remove.Content).Foreground = design.Brush("on-clay");
        remove.Resources["ButtonBackgroundDisabled"] = design.Brush("clay");
        remove.Resources["ButtonBorderBrushDisabled"] = design.Brush("clay");
        remove.Click += async (_, _) => { if (await groups.DeleteFolderAsync(folderId)) deleted(); };
        actions.Children.Add(cancel); actions.Children.Add(remove); Footer.Children.Add(actions); Footer.Visibility = Visibility.Visible;
        refresh = () =>
        {
            remove.IsEnabled = groups.CanDeleteFolder(folderId); remove.Opacity = remove.IsEnabled ? 1 : .5; cancel.IsEnabled = !groups.Busy;
            error.Text = (groups.FolderDeleteUnconfirmed(folderId) ? design.L("结果未能确认，不会自动重试。关闭后请先检查分组列表。", "The result could not be confirmed and will not be retried. Close and check the group list first.") + "\n" : "") + groups.FolderFailureDetail;
            error.Visibility = groups.Error == null ? Visibility.Collapsed : Visibility.Visible;
        };
        groups.Changed += refresh; refresh();
    }
    public override void HandleEscape() { if (!groups.Busy) base.HandleEscape(); }
    public override void Dispose() { groups.Changed -= refresh; base.Dispose(); }
}

internal sealed class NativeCapabilityDetailPage : NativeActionPage
{
    public NativeCapabilityDetailPage(INativeSettingsClient client, string project, string section, WispDesign design, Action close)
        : base(design, section == "skills" ? "技能" : section == "connections" ? "连接" : "记忆文件", new WorkspaceActionModel(), close)
    {
        async Task Load() => await State.RunAsync(() => client.InvokeAsync(section == "skills" ? "list_skills" : section == "connections" ? "list_mcp_connections" : "get_memory_view",
            section == "memory" ? new() { ["project_id"] = project } : new(), project), value =>
        {
            Results.Children.Clear();
            var rows = section == "skills" ? value as JsonArray : value?[section == "connections" ? "connections" : "files"] as JsonArray;
            foreach (var row in rows ?? [])
            {
                var title = row?["name"]?.GetValue<string>() ?? row?["path"]?.GetValue<string>() ?? row?["id"]?.GetValue<string>() ?? "";
                var enabled = row?["enabled"]?.GetValue<bool>();
                Results.Children.Add(new TextBlock { Text = title + (enabled == null ? "" : enabled.Value ? " · 已启用" : " · 未启用"), TextWrapping = TextWrapping.Wrap });
                if (row?["description"] is JsonValue description) Results.Children.Add(Mute(description.GetValue<string>()));
                if (section == "memory" && row?["content"] is JsonValue content) Results.Children.Add(new TextBox { AcceptsReturn = true, Text = content.GetValue<string>(), IsReadOnly = true, TextWrapping = TextWrapping.Wrap });
            }
            if (rows == null || rows.Count == 0) Results.Children.Add(Mute("暂无记录"));
        });
        Form.Children.Add(Mute("当前显示项目能力详情。编辑配置请使用桌面版对应设置页。"));
        Form.Children.Add(Button("刷新", Load)); _ = Load();
    }
}
