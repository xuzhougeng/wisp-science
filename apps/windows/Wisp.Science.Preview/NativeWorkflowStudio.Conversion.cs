using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed partial class NativeWorkflowStudio
{
    private Action? refreshConversionDialog;
    private void RefreshConversion() { if (!closed) { refreshConversionDialog?.Invoke(); UpdateState(); } }
    public void OpenConversion()
    {
        if (refreshConversionDialog != null || closed) return;
        var state = ConversionState;
        var overlay = new Grid(); NativeDialogVisuals.Backdrop(overlay, design);
        var card = new Grid { Background = design.Brush("bg-elev"), CornerRadius = new CornerRadius(16), TabFocusNavigation = KeyboardNavigationMode.Cycle };
        card.RowDefinitions.Add(new() { Height = GridLength.Auto }); card.RowDefinitions.Add(new() { Height = GridLength.Auto });
        card.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) }); card.RowDefinitions.Add(new() { Height = GridLength.Auto });
        var head = new Grid { Padding = new Thickness(26, 24, 26, 18), ColumnSpacing = 16 };
        head.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); head.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var intro = new StackPanel { Spacing = 8 }; intro.Children.Add(Text(T("workflow_studio.portfolio.title"), 20, true)); intro.Children.Add(Hint(T("workflow_studio.portfolio.subtitle"))); head.Children.Add(intro);
        var close = Icon(T("workflow_studio.portfolio.close"), "close", CloseDialog); Grid.SetColumn(close, 1); head.Children.Add(close); card.Children.Add(head);
        var steps = new NativeActionWrap { Margin = new Thickness(26, 0, 26, 18) }; Grid.SetRow(steps, 1); card.Children.Add(steps);
        var workspace = new Grid(); workspace.ColumnDefinitions.Add(new() { Width = new GridLength(.85, GridUnitType.Star) }); workspace.ColumnDefinitions.Add(new() { Width = new GridLength(1.25, GridUnitType.Star) });
        workspace.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) }); workspace.RowDefinitions.Add(new() { Height = GridLength.Auto });
        var inputs = new StackPanel { Spacing = 12, Padding = new Thickness(26, 22, 26, 22) };
        var review = new StackPanel { Spacing = 16, Padding = new Thickness(26, 22, 26, 22) };
        var inputScroll = new ScrollViewer { Content = inputs, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        var reviewScroll = new ScrollViewer { Content = review, Background = design.Brush("bg-sunken"), HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        var inputFrame = new Border { Child = inputScroll, BorderBrush = design.Brush("border"), BorderThickness = new Thickness(0, 1, 1, 0) };
        workspace.Children.Add(inputFrame); Grid.SetColumn(reviewScroll, 1); workspace.Children.Add(reviewScroll);
        var wholeScroll = new ScrollViewer { Content = workspace, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch, VerticalContentAlignment = VerticalAlignment.Stretch };
        Grid.SetRow(wholeScroll, 2); card.Children.Add(wholeScroll);
        var request = Input(T("workflow_studio.portfolio.request"), state.Request, state.SetRequest, true); request.PlaceholderText = T("workflow_studio.portfolio.request_hint"); request.MinHeight = 118; inputs.Children.Add(request);
        inputs.Children.Add(Text(T("workflow_studio.source_skill"), 12, true));
        var sourceMode = new Grid { ColumnSpacing = 8 }; sourceMode.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); sourceMode.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        var auto = Button(T("workflow_studio.portfolio.source_auto"), () => state.SetAutomatic(true));
        var manual = Button(T("workflow_studio.portfolio.source_manual"), () => state.SetAutomatic(false)); sourceMode.Children.Add(auto); Grid.SetColumn(manual, 1); sourceMode.Children.Add(manual); inputs.Children.Add(sourceMode);
        foreach (var (button, kind, key) in new[] { (auto, "sparkles", "source_auto"), (manual, "book", "source_manual") })
        { var content = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 }; content.Children.Add(design.Icon(kind, 14)); content.Children.Add(Text(T("workflow_studio.portfolio." + key), 12)); button.Content = content; button.HorizontalAlignment = HorizontalAlignment.Stretch; }
        var autoHint = Hint(T("workflow_studio.portfolio.source_auto_help")); inputs.Children.Add(autoHint);
        var legacyHint = Hint(T("workflow_studio.portfolio.legacy_source")); inputs.Children.Add(legacyHint);
        var manualPanel = new StackPanel { Spacing = 8 }; var sourceList = new StackPanel { Spacing = 5 }; var count = Hint("");
        void Sources()
        {
            sourceList.Children.Clear();
            foreach (var skill in state.Skills.Where(s => (S(s, "id") + " " + S(s, "name")).Contains(state.Search, StringComparison.OrdinalIgnoreCase)))
            {
                var id = S(skill, "id"); var check = new CheckBox { Content = Text(S(skill, "name"), 12), IsChecked = state.Sources.Contains(id), IsEnabled = !state.Busy && (state.Sources.Count < 8 || state.Sources.Contains(id)) };
                AutomationProperties.SetName(check, S(skill, "name")); check.Checked += (_, _) => state.SetSource(id, true); check.Unchecked += (_, _) => state.SetSource(id, false); sourceList.Children.Add(check);
            }
            if (sourceList.Children.Count == 0) sourceList.Children.Add(Hint(T("workflow_studio.portfolio.source_no_match")));
            count.Text = F("workflow_studio.portfolio.source_count", ("count", state.Sources.Count.ToString()));
        }
        var search = Input(T("workflow_studio.portfolio.source_search"), state.Search, value => { state.Search = value; Sources(); }); manualPanel.Children.Add(search);
        manualPanel.Children.Add(new ScrollViewer { Content = sourceList, MaxHeight = 220, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled }); manualPanel.Children.Add(count); inputs.Children.Add(manualPanel);
        if (state.Skills.Length == 0) inputs.Children.Add(Hint(T("workflow_studio.portfolio.no_sources")));
        inputs.Children.Add(Hint(T("workflow_studio.portfolio.source_support")));
        var models = state.Models.Select(m => (S(m, "id"), m["external"]?.GetValue<bool>() == true ? F("workflow_studio.portfolio.model_external", ("model", S(m, "label"))) : S(m, "label"))).ToArray();
        var modelChoice = Choice(T("workflow_studio.portfolio.model"), state.ModelId, models.Length > 0 ? models : [("", T("workflow_studio.portfolio.no_models"))], state.SetModel); inputs.Children.Add(modelChoice);
        inputs.Children.Add(Hint(T("workflow_studio.portfolio.model_help"))); if (models.Length == 0) inputs.Children.Add(Hint(T("workflow_studio.portfolio.no_models_help")));
        if (state.ProjectId.Length == 0) inputs.Children.Add(Hint(L("请先打开一个项目，再转换工作流。", "Open a project before converting a workflow.")));
        var footer = new Grid { Padding = new Thickness(26, 16, 26, 16), ColumnSpacing = 16, RowSpacing = 10, BorderBrush = design.Brush("border"), BorderThickness = new Thickness(0, 1, 0, 0) };
        footer.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); footer.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); footer.RowDefinitions.Add(new() { Height = GridLength.Auto }); footer.RowDefinitions.Add(new() { Height = GridLength.Auto });
        var footnote = Hint(T("workflow_studio.portfolio.footer")); footer.Children.Add(footnote);
        var actions = new NativeActionWrap { RightAligned = true }; Grid.SetColumn(actions, 1); footer.Children.Add(actions);
        var background = Button(T("workflow_studio.portfolio.close"), CloseDialog); actions.Children.Add(background);
        var generate = Button(T("workflow_studio.portfolio.generate"), async () => { await state.GenerateAsync(conversionClient, design.Localization); }, true); actions.Children.Add(generate);
        var use = Button(T("workflow_studio.portfolio.edit_studio"), () =>
        {
            if (state.Result is not { } result) return;
            void Adopt() { if (model.UseConversion(result, state.LegacyTemplateId)) { CloseDialogs(); state.Consume(); autoFit = true; Render(); } }
            if (model.HasChanges) Confirm(L("用转换草稿替换尚未保存的工作流？", "Replace the unsaved workflow with this converted draft?"), Adopt); else Adopt();
        }, true); actions.Children.Add(use); Grid.SetRow(footer, 3); card.Children.Add(footer);
        var host = new ContentControl { Content = card, HorizontalContentAlignment = HorizontalAlignment.Stretch, VerticalContentAlignment = VerticalAlignment.Stretch, HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center };
        overlay.Children.Add(host);
        void Resize()
        {
            var narrow = ActualWidth <= 720; host.Width = Math.Max(0, Math.Min(1080, ActualWidth - (narrow ? 20 : 40))); host.Height = Math.Max(0, Math.Min(800, ActualHeight - (narrow ? 20 : 40)));
            wholeScroll.VerticalScrollBarVisibility = narrow ? ScrollBarVisibility.Auto : ScrollBarVisibility.Disabled;
            workspace.ColumnDefinitions[0].Width = new GridLength(narrow ? 1 : .85, GridUnitType.Star); workspace.ColumnDefinitions[1].Width = narrow ? new GridLength(0) : new GridLength(1.25, GridUnitType.Star);
            workspace.RowDefinitions[0].Height = narrow ? GridLength.Auto : new GridLength(1, GridUnitType.Star);
            Grid.SetColumn(reviewScroll, narrow ? 0 : 1); Grid.SetRow(reviewScroll, narrow ? 1 : 0);
            inputScroll.VerticalScrollBarVisibility = reviewScroll.VerticalScrollBarVisibility = narrow ? ScrollBarVisibility.Disabled : ScrollBarVisibility.Auto;
            inputFrame.BorderThickness = narrow ? new Thickness(0, 1, 0, 1) : new Thickness(0, 1, 1, 0);
            Grid.SetColumnSpan(footnote, narrow ? 2 : 1); Grid.SetRow(actions, narrow ? 1 : 0); Grid.SetColumn(actions, narrow ? 0 : 1); Grid.SetColumnSpan(actions, narrow ? 2 : 1);
        }
        TextBlock? elapsedText = null; var timer = new DispatcherTimer { Interval = TimeSpan.FromSeconds(1) };
        void Tick() { if (elapsedText != null) elapsedText.Text = F("workflow_studio.conversion.elapsed", ("time", $"{(int)state.Elapsed.TotalMinutes}:{state.Elapsed.Seconds:00}")); }
        timer.Tick += (_, _) => Tick();
        refreshConversionDialog = () =>
        {
            inputScroll.IsEnabled = !state.Busy; generate.IsEnabled = state.Ready; use.Visibility = state.Result == null ? Visibility.Collapsed : Visibility.Visible;
            background.Content = T(state.Busy ? "workflow_studio.conversion.background" : "workflow_studio.portfolio.close");
            generate.Content = T(state.Busy ? "workflow_studio.portfolio.planning" : state.Result != null ? "workflow_studio.portfolio.regenerate" : "workflow_studio.portfolio.generate");
            AutomationProperties.SetName(background, (string)background.Content); AutomationProperties.SetName(generate, (string)generate.Content);
            sourceMode.Visibility = state.Legacy ? Visibility.Collapsed : Visibility.Visible; legacyHint.Visibility = state.Legacy ? Visibility.Visible : Visibility.Collapsed;
            manualPanel.Visibility = !state.Legacy && !state.AutoSources ? Visibility.Visible : Visibility.Collapsed; autoHint.Visibility = !state.Legacy && state.AutoSources ? Visibility.Visible : Visibility.Collapsed;
            foreach (var (button, selected) in new[] { (auto, state.AutoSources), (manual, !state.AutoSources) }) { button.Background = design.Brush(selected ? "bg-sunken" : "bg-elev"); button.BorderBrush = design.Brush(selected ? "clay" : "border-strong"); }
            Sources();
            steps.Children.Clear(); var active = state.Busy ? 1 : state.Result != null ? 2 : 0;
            foreach (var (key, index) in new[] { "source", "convert", "review" }.Select((key, i) => (key, i)))
            {
                var step = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, Margin = new Thickness(0, 0, 20, 0) };
                var number = Text((index + 1).ToString(), 11); number.HorizontalAlignment = HorizontalAlignment.Center; number.VerticalAlignment = VerticalAlignment.Center; number.Foreground = design.Brush(index == active ? "clay" : "text-faint");
                step.Children.Add(new Border { Child = number, Width = 23, Height = 23, CornerRadius = new CornerRadius(12), BorderThickness = new Thickness(1), BorderBrush = design.Brush(index == active ? "clay" : "border"), Background = design.Brush(index == active ? "bg-sunken" : "bg-elev") });
                var label = Text(T("workflow_studio.portfolio.step_" + key), 12, index == active); label.VerticalAlignment = VerticalAlignment.Center; label.Foreground = design.Brush(index == active ? "text" : "text-faint"); step.Children.Add(label); steps.Children.Add(step);
            }
            review.Children.Clear(); elapsedText = null;
            reviewScroll.VerticalContentAlignment = !state.Busy && state.Result == null && state.Error == null ? VerticalAlignment.Center : VerticalAlignment.Top;
            review.VerticalAlignment = reviewScroll.VerticalContentAlignment;
            if (state.Error != null) { review.Children.Add(Text(T("workflow_studio.portfolio.error"), 15, true)); review.Children.Add(Hint(state.Error)); }
            if (state.Busy)
            { var status = Text(T(state.StatusKey), 14, true); AutomationProperties.SetLiveSetting(status, AutomationLiveSetting.Polite); review.Children.Add(status); elapsedText = Hint(""); review.Children.Add(elapsedText); review.Children.Add(new ProgressBar { IsIndeterminate = true }); review.Children.Add(Hint(T("workflow_studio.conversion.background_hint"))); Tick(); }
            else if (state.Result is { } result) ConversionReview(review, result);
            else
            {
                var icon = design.Icon("branch", 30, design.ColorText("clay")); icon.HorizontalAlignment = HorizontalAlignment.Center; review.Children.Add(icon);
                var title = Text(T("workflow_studio.portfolio.empty_title"), 16, true); title.TextAlignment = TextAlignment.Center; review.Children.Add(title);
                var hint = Hint(T("workflow_studio.portfolio.empty_help")); hint.MaxWidth = 400; hint.HorizontalAlignment = HorizontalAlignment.Center; hint.TextAlignment = TextAlignment.Center; review.Children.Add(hint);
                var checklist = new StackPanel { Spacing = 12, HorizontalAlignment = HorizontalAlignment.Center, Margin = new Thickness(0, 12, 0, 0) };
                foreach (var (key, kind) in new[] { ("review_instructions", "list"), ("review_permissions", "shield"), ("review_outputs", "doc") })
                { var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 10 }; row.Children.Add(design.Icon(kind, 15, design.ColorText("clay"))); row.Children.Add(Hint(T("workflow_studio.portfolio." + key))); checklist.Children.Add(row); }
                review.Children.Add(checklist);
            }
        };
        overlay.Tag = (Action)(() => { timer.Stop(); refreshConversionDialog = null; state.Show(false); });
        overlay.Loaded += (_, _) => { Resize(); close.Focus(FocusState.Programmatic); }; overlay.SizeChanged += (_, _) => Resize();
        overlay.PointerPressed += (_, e) => { if (ReferenceEquals(e.OriginalSource, overlay)) CloseDialog(); };
        dialogs.Add(overlay); root.Children.Add(overlay); timer.Start(); state.Show(true); UpdateState();
    }
    private void ConversionReview(StackPanel panel, JsonObject result)
    {
        var proposal = result["proposal"]!; var plan = result["plan"]!; var tasks = (proposal["tasks"] as JsonArray)?.OfType<JsonObject>().ToArray() ?? [];
        var sources = NativeWorkflowConversionModel.SourceIds(result);
        panel.Children.Add(Badge(T("workflow_studio.portfolio.draft_ready"), true)); panel.Children.Add(Text(S(proposal, "goal"), 18, true));
        panel.Children.Add(Hint(F("workflow_studio.portfolio.summary", ("tasks", tasks.Length.ToString()), ("skills", sources.Length.ToString()), ("model", S(plan, "planner_model_label")))));
        panel.Children.Add(Text(T("workflow_studio.portfolio.provenance"), 13, true)); var tags = new NativeActionWrap(); foreach (var source in sources) tags.Children.Add(Badge(source)); panel.Children.Add(tags);
        panel.Children.Add(Hint(T("workflow_studio.portfolio.provenance_help"))); var provenance = new StackPanel { Spacing = 8 }; provenance.Children.Add(Hint(S(plan, "rationale")));
        if (S(plan, "source_sha256") is { Length: > 0 } hash) provenance.Children.Add(Hint("SHA-256: " + hash));
        panel.Children.Add(new Expander { Header = Text(T("workflow_studio.portfolio.source_details"), 12), Content = provenance, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch });
        panel.Children.Add(Hint(T("workflow_studio.portfolio.validated_unbudgeted")));
        foreach (var (task, index) in tasks.Select((task, i) => (task, i)))
        {
            var body = new StackPanel { Spacing = 9 }; body.Children.Add(Text($"{index + 1}   " + S(task, "id"), 13, true)); var deps = NativeWorkflowGraph.Dependencies(task);
            body.Children.Add(Hint(deps.Length == 0 ? T("workflow_studio.portfolio.no_dependencies") : F("workflow_studio.portfolio.task_after", ("tasks", string.Join(", ", deps)))));
            body.Children.Add(Text(T("workflow_studio.portfolio.node_instruction"), 12, true)); body.Children.Add(Text(S(task, "instruction"), 12));
            body.Children.Add(Text(T("workflow_studio.portfolio.node_permissions"), 12, true)); var caps = NativeWorkflowOptions.CapabilityIds(task); var permissions = new NativeActionWrap(); foreach (var cap in caps) permissions.Children.Add(Badge(cap));
            if (caps.Length == 0) body.Children.Add(Hint(T("workflow_studio.portfolio.no_permissions"))); else body.Children.Add(permissions);
            body.Children.Add(Text(T("workflow_studio.portfolio.node_output"), 12, true));
            if (task["output_schema"] is { } schema) body.Children.Add(new Expander { Header = Text(T("workflow_studio.portfolio.view_contract"), 12), Content = Text(schema.ToJsonString(new() { WriteIndented = true }), 11), HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch });
            else body.Children.Add(Hint(T("workflow_studio.portfolio.no_contract")));
            var card = design.Card(body); card.Padding = new Thickness(14); panel.Children.Add(card);
        }
    }
}
