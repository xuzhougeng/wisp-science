using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed partial class NativeWorkflowStudio
{
    private (StackPanel Body, NativeActionWrap Footer) Dialog(string title, double width = 760, bool nodeEditor = false, Action? onClosed = null)
    {
        if (dialogs.LastOrDefault()?.Children.OfType<ContentControl>().FirstOrDefault() is ContentControl previous) previous.IsEnabled = false;
        var overlay = new Grid { Tag = onClosed };
        NativeDialogVisuals.Backdrop(overlay, design);
        var card = new Grid { Background = design.Brush("bg-elev"), CornerRadius = new CornerRadius(16), TabFocusNavigation = KeyboardNavigationMode.Cycle };
        card.RowDefinitions.Add(new() { Height = GridLength.Auto }); card.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) }); card.RowDefinitions.Add(new() { Height = GridLength.Auto });
        var heading = new Grid { Padding = new Thickness(22, 18, 22, 14) };
        heading.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); heading.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var caption = Text(title, 18, true); heading.Children.Add(caption); var close = Icon(L("关闭", "Close ") + title, "close", CloseDialog); Grid.SetColumn(close, 1); heading.Children.Add(close);
        var body = new StackPanel { Spacing = 14, Padding = nodeEditor ? new Thickness(24, 4, 34, 24) : new Thickness(22, 8, 22, 20) };
        var scroll = new ScrollViewer { HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        if (!nodeEditor) { card.Children.Add(heading); scroll.Content = body; }
        else
        {
            // The node editor is one scrolling sheet. Preserve space for the heading
            // at rest, but pin only the heading itself once its top inset has scrolled away.
            var sticky = new TranslateTransform(); heading.RenderTransform = sticky;
            heading.VerticalAlignment = VerticalAlignment.Top; heading.Background = design.Brush("bg-elev");
            heading.Margin = new Thickness(0, 24, 0, 0); caption.VerticalAlignment = VerticalAlignment.Center;
            void Gutter()
            {
                var right = scroll.ScrollableHeight > 0 ? 34 : 24;
                if (heading.Padding.Right != right) heading.Padding = new Thickness(24, 12, right, 12);
                if (body.Padding.Right != right) body.Padding = new Thickness(24, 4, right, 24);
            }
            heading.SizeChanged += (_, e) => body.Margin = new Thickness(0, 24 + e.NewSize.Height + 16, 0, 0);
            scroll.RegisterPropertyChangedCallback(ScrollViewer.ScrollableHeightProperty, (_, _) => Gutter());
            scroll.ViewChanged += (_, _) => sticky.Y = Math.Max(0, scroll.VerticalOffset - 24);
            scroll.Content = new Grid { Children = { body, heading } }; Gutter();
            overlay.Tapped += (_, e) =>
            {
                if (ReferenceEquals(e.OriginalSource, overlay) && dialogs.LastOrDefault() == overlay)
                { e.Handled = true; CloseDialog(); }
            };
        }
        Grid.SetRow(scroll, 1); card.Children.Add(scroll);
        var footer = new NativeActionWrap { HorizontalAlignment = HorizontalAlignment.Right, Margin = new Thickness(22, 0, 22, 20) }; Grid.SetRow(footer, 2); card.Children.Add(footer);
        footer.Visibility = nodeEditor ? Visibility.Collapsed : Visibility.Visible;
        var host = new ContentControl { Content = card, HorizontalContentAlignment = HorizontalAlignment.Stretch, VerticalContentAlignment = VerticalAlignment.Stretch, HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center };
        overlay.Children.Add(host);
        void Resize() { host.Width = Math.Max(0, Math.Min(width, ActualWidth - 40)); host.MaxHeight = Math.Max(0, ActualHeight - (nodeEditor ? 64 : 40)); body.Width = host.Width; }
        overlay.Loaded += (_, _) => { Resize(); close.Focus(FocusState.Programmatic); }; overlay.SizeChanged += (_, _) => Resize();
        dialogs.Add(overlay); root.Children.Add(overlay); UpdateState(); return (body, footer);
    }
    private void CloseDialog()
    {
        if (dialogs.Count == 0) return;
        var dialog = dialogs[^1]; dialogs.RemoveAt(dialogs.Count - 1); root.Children.Remove(dialog);
        if (dialogs.LastOrDefault()?.Children.OfType<ContentControl>().FirstOrDefault() is ContentControl previous) previous.IsEnabled = true;
        (dialog.Tag as Action)?.Invoke();
        UpdateState();
    }
    private void CloseDialogs() { while (dialogs.Count > 0) CloseDialog(); }
    private void Confirm(string message, Action confirmed)
    {
        var (body, footer) = Dialog(L("确认", "Confirm"), 460); body.Children.Add(Text(message));
        footer.Children.Add(Button(T("workflow_studio.delete_cancel"), CloseDialog)); footer.Children.Add(Button(L("确认", "Confirm"), () => { CloseDialog(); confirmed(); }, true));
    }
    private void OpenNew()
    {
        var (body, _) = Dialog(T("workflow_studio.new"));
        var options = new Grid { ColumnSpacing = 12, RowSpacing = 12 };
        for (var i = 0; i < 3; i++) { options.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); options.RowDefinitions.Add(new() { Height = GridLength.Auto }); }
        void Option(int index, string title, string hint, string icon, Action action)
        {
            var copy = new StackPanel { Spacing = 12, Padding = new Thickness(4) };
            var symbol = design.Icon(icon, 24, design.ColorText("clay")); symbol.HorizontalAlignment = HorizontalAlignment.Left;
            copy.Children.Add(symbol); copy.Children.Add(Text(title, 14, true)); copy.Children.Add(Hint(hint));
            var button = Button(title, action); button.Content = copy; button.HorizontalAlignment = HorizontalAlignment.Stretch; button.HorizontalContentAlignment = HorizontalAlignment.Stretch;
            Grid.SetColumn(button, index); options.Children.Add(button);
        }
        Option(0, T("workflow_studio.from_scratch"), T("workflow_studio.from_scratch_help"), "plus", () => { model.New(); CloseDialogs(); autoFit = true; Render(); });
        Option(1, T("workflow_studio.use_template"), T("workflow_studio.use_template_help"), "copy", OpenTemplatePicker);
        Option(2, T("workflow_studio.from_skills"), T("workflow_studio.from_skills_help"), "branch", () => { CloseDialogs(); if (ConversionState.RequestId == null) ConversionState.SetLegacy(null); OpenConversion(); });
        options.SizeChanged += (_, e) =>
        {
            var narrow = e.NewSize.Width < 580; options.RowSpacing = narrow ? 12 : 0;
            for (var i = 0; i < options.Children.Count; i++) { var child = (FrameworkElement)options.Children[i]; Grid.SetColumn(child, narrow ? 0 : i); Grid.SetRow(child, narrow ? i : 0); Grid.SetColumnSpan(child, narrow ? 3 : 1); }
        };
        body.Children.Add(options);
    }
    private void OpenTemplatePicker()
    {
        var (body, _) = Dialog(T("workflow_studio.use_template"));
        foreach (var template in model.Templates)
        {
            var button = Button(S(template, "name"), () => { model.New(template); CloseDialogs(); autoFit = true; Render(); });
            button.HorizontalAlignment = HorizontalAlignment.Stretch; button.HorizontalContentAlignment = HorizontalAlignment.Left; body.Children.Add(button);
        }
        if (model.Templates.Length == 0) body.Children.Add(Hint(L("尚无模板，可以从空白工作流开始。", "No templates yet. Start with a blank workflow.")));
    }
    private FrameworkElement NodeDisclosure(string title, FrameworkElement content, bool framed = true, TextBlock? summary = null)
    {
        var toggle = new Microsoft.UI.Xaml.Controls.Primitives.ToggleButton { HorizontalAlignment = HorizontalAlignment.Stretch,
            HorizontalContentAlignment = HorizontalAlignment.Stretch, MinHeight = framed ? 32 : 0, MinWidth = 0,
            Padding = framed ? new Thickness(0, 6, 0, 6) : new Thickness(0), BorderThickness = new Thickness(0),
            Background = design.Brush("bg-elev"), Foreground = design.Brush("text-muted") };
        foreach (var state in new[] { "PointerOver", "Pressed", "Checked", "CheckedPointerOver", "CheckedPressed" })
        {
            toggle.Resources["ToggleButtonBackground" + state] = design.Brush("bg-elev");
            toggle.Resources["ToggleButtonForeground" + state] = design.Brush("text-muted");
        }
        var header = new StackPanel { Spacing = 8 };
        var line = new Grid { ColumnSpacing = 3 };
        line.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); line.ColumnDefinitions.Add(new());
        var arrow = design.Icon("chevron-right", 11, design.ColorText("text-muted")); arrow.VerticalAlignment = VerticalAlignment.Center;
        arrow.RenderTransformOrigin = new Windows.Foundation.Point(.5f, .5f); line.Children.Add(arrow);
        var caption = Text(title, 13, summary != null || !framed); caption.Foreground = design.Brush("text-muted");
        Grid.SetColumn(caption, 1); line.Children.Add(caption); header.Children.Add(line);
        if (summary != null) header.Children.Add(summary);
        toggle.Content = header; AutomationProperties.SetName(toggle, title);
        content.Visibility = Visibility.Collapsed;
        toggle.Checked += (_, _) => { arrow.RenderTransform = new RotateTransform { Angle = 90 }; content.Visibility = Visibility.Visible; };
        toggle.Unchecked += (_, _) => { arrow.RenderTransform = null; content.Visibility = Visibility.Collapsed; };
        var section = new StackPanel { Margin = new Thickness(0, 8, 0, 0), Children = { toggle, content } };
        if (!framed) return section;
        section.Margin = new Thickness(0);
        return new Border { Margin = new Thickness(0, 8, 0, 0), Padding = new Thickness(7), CornerRadius = new CornerRadius(7),
            BorderBrush = design.Brush("border"), BorderThickness = new Thickness(1), Background = design.Brush("bg-elev"), Child = section };
    }
    private Border NodeCheckOption(CheckBox check) => new() { Child = check, Padding = new Thickness(7), CornerRadius = new CornerRadius(6),
        BorderBrush = design.Brush("border"), BorderThickness = new Thickness(1), Background = design.Brush("bg-elev") };
    private void OpenTask(long? existingKey, JsonObject source)
    {
        if (model.Busy) return;
        var taskKey = existingKey ?? model.AddTaskDraft(source); if (taskKey < 0 || model.TaskForKey(taskKey) is not { } task) return;
        model.SelectTaskKey(taskKey);
        var (body, _) = Dialog(T("workflow_studio.edit_node"), 820, nodeEditor: true, onClosed: Render);
        var error = Text(""); error.Foreground = design.Brush("clay-strong"); error.Visibility = Visibility.Collapsed; body.Children.Add(error);
        void RefreshError()
        {
            var id = S(task, "id");
            error.Text = model.TaskInputError(taskKey) ?? model.Error ?? (string.IsNullOrWhiteSpace(id) ? T("workflow_studio.task_id_required")
                : model.Tasks.Count(row => S(row, "id").Trim() == id.Trim()) > 1 ? L("任务 ID 必须唯一。", "Task IDs must be unique.") : "");
            error.Visibility = error.Text.Length == 0 ? Visibility.Collapsed : Visibility.Visible;
        }
        void Commit() { model.UpdateTaskDraft(taskKey, task); DrawGraph(); RefreshError(); }
        FrameworkElement Labeled(Control control)
        {
            string title;
            if (control is TextBox input)
            {
                title = (string)input.Header; input.Header = null;
                input.MinHeight = input.AcceptsReturn ? 112 : 36;
                input.TextWrapping = input.AcceptsReturn ? TextWrapping.Wrap : TextWrapping.NoWrap;
                input.Padding = new Thickness(10, 7, 10, 7);
            }
            else
            {
                var combo = (ComboBox)control; title = (string)combo.Header; combo.Header = null; combo.MinHeight = 36;
            }
            control.MinWidth = 0; design.BindTypography(control, size: 13); PreferenceFieldAppearance.Apply(control, design);
            var label = Text(title, 13); label.Foreground = design.Brush("text-muted");
            AutomationProperties.SetLabeledBy(control, label); AutomationProperties.SetName(control, title);
            return new StackPanel { Spacing = 4, Children = { label, control } };
        }
        void Lines(StackPanel parent, string key, string title)
            => parent.Children.Add(Labeled(Input(title, string.Join("\n", (task[key] as JsonArray ?? []).Select(n => n?.GetValue<string>() ?? "")), value => { task[key] = new JsonArray(value.Split('\n').Select(s => s.Trim()).Where(s => s.Length > 0).Distinct(StringComparer.Ordinal).Select(s => (JsonNode?)JsonValue.Create(s)).ToArray()); Commit(); }, true)));
        TextBox Schema(string key, string title)
        {
            var box = Input(title, model.TaskInputText(taskKey, key, task[key]?.ToJsonString(new() { WriteIndented = true }) ?? ""), value =>
            {
                string? problem = null;
                try { task[key] = string.IsNullOrWhiteSpace(value) ? null : JsonNode.Parse(value); } catch (System.Text.Json.JsonException) { problem = title + L("：请输入有效 JSON。", ": enter valid JSON."); }
                model.SetTaskInputError(taskKey, key, value, problem); Commit();
            }, true);
            box.PlaceholderText = T("agents.task.output_schema_hint"); box.IsSpellCheckEnabled = false; return box;
        }
        TextBox Number(JsonObject target, string key, string title, ulong max, bool required = false, string prefix = "")
        {
            var input = Input(title, model.TaskInputText(taskKey, prefix + key, target[key]?.ToString() ?? ""), value =>
            {
                var valid = true;
                if (!required && string.IsNullOrWhiteSpace(value)) target[key] = null;
                else if (ulong.TryParse(value, out var number) && number <= max && (!required || number > 0)) target[key] = number;
                else valid = false;
                if (valid && prefix == "budget.") task["budget"] = target.DeepClone();
                model.SetTaskInputError(taskKey, prefix + key, value, valid ? null : title + L("：请输入有效数值。", ": enter a valid number.")); Commit();
            });
            PreferenceNumberBox.ApplyUnsigned(input, design, required ? 1UL : 0UL, max);
            return input;
        }
        Grid Pair(Control a, Control b, bool taskHeader = false)
        {
            var first = a is CheckBox ? (FrameworkElement)a : Labeled(a); var second = Labeled(b);
            var grid = new Grid { ColumnSpacing = taskHeader ? 12 : 8 };
            grid.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); grid.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
            grid.Children.Add(first); Grid.SetColumn(second, 1); grid.Children.Add(second);
            if (taskHeader)
            {
                grid.RowDefinitions.Add(new() { Height = GridLength.Auto }); grid.RowDefinitions.Add(new() { Height = GridLength.Auto });
                void Arrange()
                {
                    var narrow = NativeWorkflowLayout.StackPanels(XamlRoot?.Size.Width ?? ActualWidth);
                    grid.RowSpacing = narrow ? 12 : 0;
                    Grid.SetColumnSpan(first, narrow ? 2 : 1); Grid.SetColumnSpan(second, narrow ? 2 : 1);
                    Grid.SetColumn(second, narrow ? 0 : 1); Grid.SetRow(second, narrow ? 1 : 0);
                }
                grid.Loaded += (_, _) => Arrange(); grid.SizeChanged += (_, _) => Arrange();
            }
            return grid;
        }
        IEnumerable<(string Id, string Label)> References(JsonObject[] rows, string current, Func<JsonObject, string> getLabel, string? empty = null, bool preserveMissing = true)
        {
            var options = new List<(string, string)> { ("", empty ?? T("agents.task.auto")) };
            options.AddRange(rows.Select(row => (S(row, "id"), getLabel(row))));
            if (preserveMissing && current.Length > 0 && rows.All(row => S(row, "id") != current)) options.Add((current, current + " · " + T("runtime.unavailable")));
            return options;
        }
        var kindFields = new StackPanel { Spacing = 14 }; var advanced = new StackPanel { Spacing = 8, Padding = new Thickness(0, 6, 0, 0) };
        var advancedGroup = NodeDisclosure(T("agents.task.advanced"), advanced, framed: false);
        var incoming = new NativeActionWrap { Gap = 5, VerticallyCentered = true };
        var dependencies = new NativeActionWrap { Gap = 5, VerticallyCentered = true };
        ComboBox? activityInput = null; var refreshingDependencies = false; var dependencyChecks = new Dictionary<string, CheckBox>(StringComparer.Ordinal);
        void ChangeDependency(string id, bool enabled)
        {
            if (refreshingDependencies) return;
            if (model.SetDraftDependency(taskKey, id, enabled) && model.TaskForKey(taskKey) is { } changed)
            {
                task["depends_on"] = changed["depends_on"]?.DeepClone();
                if (task["run_activity"] is JsonObject activity) activity["input_task_id"] = S(changed["run_activity"], "input_task_id");
                DrawGraph();
            }
            RefreshDependencies(); RefreshError();
        }
        void RefreshDependencies()
        {
            var ids = NativeWorkflowGraph.Dependencies(task); incoming.Children.Clear();
            if (ids.Length == 0) { var rootLabel = Text(T("workflow_studio.graph_root"), 9.5); rootLabel.Foreground = design.Brush("text-faint"); incoming.Children.Add(rootLabel); }
            else foreach (var id in ids)
            {
                var chip = Button(T("workflow_studio.graph_remove_edge") + " " + id, () => ChangeDependency(id, false));
                chip.Padding = new Thickness(8, 4, 8, 4); chip.MinHeight = chip.MinWidth = 0; chip.CornerRadius = new CornerRadius(20);
                var content = new Grid { ColumnSpacing = 6 };
                content.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); content.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
                var name = MonoText(id, 11); name.Foreground = design.Brush("text-muted"); name.TextWrapping = TextWrapping.NoWrap; name.TextTrimming = TextTrimming.CharacterEllipsis;
                var cross = design.Icon("close", 14, design.ColorText("text-muted")); Grid.SetColumn(cross, 1);
                content.Children.Add(name); content.Children.Add(cross); chip.Content = content; incoming.Children.Add(chip);
            }
            refreshingDependencies = true;
            foreach (var (id, check) in dependencyChecks) check.IsChecked = ids.Contains(id);
            if (activityInput == null) { refreshingDependencies = false; return; }
            activityInput.Items.Clear();
            activityInput.Items.Add(new ComboBoxItem { Tag = "", Content = T("agents.run_activity_input_choose") });
            foreach (var id in ids) activityInput.Items.Add(new ComboBoxItem { Tag = id, Content = id });
            activityInput.SelectedItem = activityInput.Items.Cast<ComboBoxItem>().FirstOrDefault(item => (string)item.Tag == S(task["run_activity"], "input_task_id"));
            refreshingDependencies = false;
        }
        void RenderKindFields()
        {
            kindFields.Children.Clear(); advanced.Children.Clear(); activityInput = null;
            var run = S(task, "task_kind") == "run_activity"; advancedGroup.Visibility = run ? Visibility.Collapsed : Visibility.Visible;
            kindFields.Margin = new Thickness(0, run ? 0 : 8, 0, 0);
            if (run)
            {
                // Existing host fields stay on the clone; changing type creates the same defaults as WebView.
                if (task["run_activity"] is not JsonObject) NativeWorkflowStudioModel.ChangeTaskKind(task, "run_activity");
                var activity = task["run_activity"]!.AsObject();
                var fields = new StackPanel { Spacing = 8 };
                var group = new Grid();
                group.Children.Add(new Border { Margin = new Thickness(0, 7, 0, 0), Padding = new Thickness(8, 16, 8, 8),
                    CornerRadius = new CornerRadius(8), BorderBrush = design.Brush("border"), BorderThickness = new Thickness(1), Child = fields });
                var legend = Text(T("agents.run_activity"), 10.5); legend.Foreground = design.Brush("text-muted");
                group.Children.Add(new Border { Child = legend, Background = design.Brush("bg-elev"), Padding = new Thickness(4, 0, 4, 0),
                    Margin = new Thickness(8, 0, 0, 0), VerticalAlignment = VerticalAlignment.Top, HorizontalAlignment = HorizontalAlignment.Left });
                kindFields.Children.Add(group);
                var help = Text(T("agents.run_activity_help"), 14); help.Margin = new Thickness(0, 0, 0, 8); fields.Children.Add(help);
                var activityKind = Input(T("agents.run_activity_kind"), S(activity, "activity"), _ => { }); activityKind.IsReadOnly = true;
                var context = S(activity, "context_id"); var contexts = new List<(string, string)> { ("local", "Local") };
                if (context.Length > 0 && context != "local") contexts.Add((context, context));
                fields.Children.Add(Pair(activityKind, Choice(T("agents.run_activity_context"), context, contexts, value => { activity["context_id"] = value; Commit(); })));
                activityInput = Choice(T("agents.run_activity_input"), S(activity, "input_task_id"), [], value => { if (!refreshingDependencies) { activity["input_task_id"] = value; Commit(); } });
                var pointer = Input(T("agents.run_activity_pointer"), S(activity, "spec_output_pointer"), _ => { }); pointer.IsReadOnly = true;
                fields.Children.Add(Pair(activityInput, pointer));
                fields.Children.Add(Pair(Number(activity, "max_candidates", T("agents.run_activity_candidates"), 50, true, "run_activity."), Number(activity, "max_wall_seconds", T("agents.run_activity_wall"), 604800, true, "run_activity.")));
                fields.Children.Add(Pair(Number(activity, "max_evaluator_seconds", T("agents.run_activity_evaluator"), 300, true, "run_activity."), Number(activity, "max_cost_microunits", T("agents.run_activity_cost"), ulong.MaxValue, true, "run_activity.")));
                RefreshDependencies(); return;
            }
            var currentSpecialist = S(task, "specialist_id");
            var specialistRows = model.Specialists.Where(row => NativeWorkflowOptions.CanAssignSpecialist(S(row, "id"))).ToArray();
            kindFields.Children.Add(Labeled(Choice(T("agents.task.specialist"), currentSpecialist,
                References(specialistRows, currentSpecialist, row => S(row, "name"), T("agents.task.temporary"), NativeWorkflowOptions.CanAssignSpecialist(currentSpecialist)),
                value => { task["specialist_id"] = value.Length == 0 ? null : JsonValue.Create(value); Commit(); })));
            var capabilities = new NativeActionWrap { Gap = 5, VerticallyCentered = true };
            var capabilitySummary = Leading(Hint(""), 1.6); capabilitySummary.TextWrapping = TextWrapping.Wrap;
            var capabilityGroup = NodeDisclosure(T("agents.task.capabilities"), capabilities, summary: capabilitySummary);
            void RefreshCapabilities() => capabilitySummary.Text = NativeWorkflowOptions.CapabilitySummary(task, model.Capabilities);
            if (model.OptionsError is { } optionsError) capabilities.Children.Add(Hint(optionsError));
            foreach (var option in model.Capabilities)
            {
                var id = S(option, "id"); var name = S(option, "display_name");
                var copy = new NativeActionWrap { Gap = 5, VerticallyCentered = true }; copy.Children.Add(Text(name, 13));
                var risk = S(option, "risk"); var riskLabel = T("agents.risk." + (risk is "read_only" or "write" or "execute" or "network" ? risk : "external"));
                var riskText = Text(System.Globalization.CultureInfo.InvariantCulture.TextInfo.ToTitleCase(riskLabel), 9); riskText.Foreground = design.Brush("text-faint"); copy.Children.Add(riskText);
                var check = PreferenceCheckBox.Create(design, name, NativeWorkflowOptions.CapabilityIds(task).Contains(id), 13);
                check.Content = copy; check.Padding = new Thickness(5, 0, 0, 0); ToolTipService.SetToolTip(check, S(option, "description"));
                check.Checked += (_, _) => { NativeWorkflowOptions.SetCapability(task, id, true); Commit(); RefreshCapabilities(); };
                check.Unchecked += (_, _) => { NativeWorkflowOptions.SetCapability(task, id, false); Commit(); RefreshCapabilities(); }; capabilities.Children.Add(NodeCheckOption(check));
            }
            RefreshCapabilities(); kindFields.Children.Add(capabilityGroup);
            var hint = Leading(Text(T("agents.task.advanced_hint"), 10.5), 1.4); hint.Foreground = design.Brush("text-faint"); advanced.Children.Add(hint);
            var modelChoice = Choice(T("agents.task.model"), S(task, "model_id"), References(model.Models, S(task, "model_id"), NativeWorkflowOptions.ModelOptionLabel), value => { task["model_id"] = value.Length == 0 ? null : JsonValue.Create(value); Commit(); });
            var executorKey = NativeWorkflowOptions.ExecutorKey(task["executor"]);
            var executorOptions = new List<(string Id, string Label)> { ("", T("agents.task.auto")) };
            foreach (var option in model.Executors)
            {
                var label = S(option, "kind") == "native" ? S(option, "display_name") : S(option, "kind") + " · " + S(option, "display_name");
                if (option["available"]?.GetValue<bool>() != true) label += " · " + T("runtime.unavailable");
                executorOptions.Add((S(option, "id"), label));
            }
            if (executorKey.Length > 0 && executorOptions.All(option => option.Id != executorKey)) executorOptions.Add((executorKey, executorKey + " · " + T("runtime.unavailable")));
            modelChoice.IsEnabled = executorKey.Length == 0 || executorKey == "native";
            var executorChoice = Choice(T("agents.task.executor"), executorKey, executorOptions, value =>
            {
                task["executor"] = NativeWorkflowOptions.Executor(value);
                var external = value.Length > 0 && value != "native"; modelChoice.IsEnabled = !external;
                if (external) { task["model_id"] = null; modelChoice.SelectedIndex = 0; }
                Commit();
            });
            foreach (var item in executorChoice.Items.Cast<ComboBoxItem>().Where(item => (string)item.Tag != ""))
            {
                var option = model.Executors.FirstOrDefault(option => S(option, "id") == (string)item.Tag);
                item.IsEnabled = option?["available"]?.GetValue<bool>() == true;
                if (option?["supported_features"] is JsonArray features) ToolTipService.SetToolTip(item, string.Join(", ", features.Select(feature => feature?.ToString())));
            }
            advanced.Children.Add(Pair(modelChoice, executorChoice));
            var isolated = PreferenceCheckBox.Create(design, T("agents.task.isolated"), task["isolated"]?.GetValue<bool>() == true, 13);
            isolated.Content = Text(T("agents.task.isolated"), 13); isolated.VerticalAlignment = VerticalAlignment.Center; isolated.Margin = new Thickness(4, 0, 0, 0);
            isolated.Checked += (_, _) => { task["isolated"] = true; Commit(); }; isolated.Unchecked += (_, _) => { task["isolated"] = false; Commit(); };
            var budget = task["budget"] as JsonObject ?? new JsonObject();
            TextBox Budget(string key, string labelKey, ulong max)
            { var input = Number(budget, key, T(labelKey), max, prefix: "budget."); input.PlaceholderText = T(labelKey + "_hint"); return input; }
            advanced.Children.Add(Pair(isolated, Budget("max_tokens", "agents.task.max_tokens", uint.MaxValue)));
            advanced.Children.Add(Pair(Budget("max_tool_calls", "agents.task.max_tools", uint.MaxValue), Budget("max_cost_microunits", "agents.task.max_cost", ulong.MaxValue)));
            var timeout = Number(task, "timeout_secs", T("agents.task.timeout"), ulong.MaxValue); timeout.PlaceholderText = T("agents.task.timeout_hint");
            advanced.Children.Add(Pair(timeout, Schema("output_schema", T("agents.task.output_schema"))));
            if (task["skill_ids"] is JsonArray { Count: > 0 }) Lines(advanced, "skill_ids", T("workflow_studio.legacy_sources"));
        }
        var typeChoice = Choice(T("agents.task.type"), S(task, "task_kind") is { Length: > 0 } kind ? kind : "agent", [("agent", T("agents.task.type_agent")), ("run_activity", T("agents.run_activity"))], value =>
        { NativeWorkflowStudioModel.ChangeTaskKind(task, value); if (value == "run_activity") model.ClearAgentInputErrors(taskKey); Commit(); RenderKindFields(); });
        body.Children.Add(Pair(Input(T("agents.task.id"), S(task, "id"), value => { task["id"] = value; Commit(); }), typeChoice, taskHeader: true));
        var instruction = Input(T("agents.task.instruction"), S(task, "instruction"), value => { task["instruction"] = value; Commit(); }, true);
        instruction.PlaceholderText = T("agents.task.instruction_ph"); body.Children.Add(Labeled(instruction)); body.Children.Add(kindFields);
        var incomingTitle = Text(T("workflow_studio.graph_incoming").ToUpperInvariant(), 11, true); incomingTitle.Foreground = design.Brush("text-faint");
        body.Children.Add(new Border { CornerRadius = new CornerRadius(7), Padding = new Thickness(8), BorderBrush = design.Brush("border"),
            BorderThickness = new Thickness(1), Background = design.Brush("bg-sunken"), Child = new StackPanel { Spacing = 5, Children = { incomingTitle, incoming } } });
        foreach (var (other, index) in model.Tasks.Select((task, index) => (task, index)).Where(row => model.TaskKeyAt(row.index) != taskKey))
        {
            var id = S(other, "id"); if (id.Trim().Length == 0 || model.Tasks.Count(row => S(row, "id").Trim() == id.Trim()) != 1) continue;
            var check = PreferenceCheckBox.Create(design, id, NativeWorkflowGraph.Dependencies(task).Contains(id), 13);
            check.Content = Text(id, 13); check.Padding = new Thickness(5, 0, 0, 0); dependencyChecks[id] = check;
            check.Checked += (_, _) => ChangeDependency(id, true); check.Unchecked += (_, _) => ChangeDependency(id, false);
            dependencies.Children.Add(NodeCheckOption(check));
        }
        if (dependencies.Children.Count == 0) dependencies.Children.Add(Hint(T("agents.task.no_dependencies")));
        body.Children.Add(NodeDisclosure(T("workflow_studio.edit_dependencies"), dependencies));
        body.Children.Add(advancedGroup); RenderKindFields(); RefreshDependencies(); RefreshError(); DrawGraph();
    }
}
