using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed class NativeExternalSessionImportPage : WorkspaceSheet
{
    private readonly WorkspaceExternalImportModel model;
    private readonly ComboBox source, provider, destination;
    private readonly TextBox query;
    private readonly Button refresh, importAll, stop, previous, next;
    private readonly TextBlock hint, progress, pageLabel;
    private readonly StackPanel rows = new() { Spacing = 4, MinHeight = 120 };
    private readonly StackPanel errors = new() { Spacing = 6 };
    private readonly ProgressBar progressBar = new() { Minimum = 0, Height = 6 };
    private readonly Func<string, string, Task> open;
    private bool rendering;
    private string sourcesKey = "";
    private string T(string key) => Design.T(key);
    private string Format(string key, params (string, string)[] values) => Design.Localization.Format(key, values);
    private string ProviderKey(string key) => (model.Provider == "claude" ? "claude." : "codex.") + key;

    public NativeExternalSessionImportPage(WorkspaceExternalImportModel model, IReadOnlyList<ProjectSummary> projects,
        WispDesign design, Func<string, string, Task> open, Action close, string initialProvider = "codex", bool chooseDestination = false)
        : base(design, chooseDestination ? design.L("导入外部会话", "Import external conversations") :
            design.T(initialProvider == "claude" ? "claude.title" : "codex.title"), close, modal: true, modalWidth: 640, lightDismiss: true)
    {
        this.model = model; this.open = open;
        model.Select(model.Project, initialProvider, "local");
        AutomationProperties.SetAutomationId(this, "external-session-import");
        ComboBox Choice(string label)
        {
            var box = new ComboBox { MinWidth = 0, Width = 180, CornerRadius = new(8) };
            AutomationProperties.SetName(box, label); design.BindTypography(box, size: 12);
            PreferenceFieldAppearance.Apply(box, design); return box;
        }
        destination = Choice(design.L("目标项目", "Destination project"));
        provider = Choice(design.L("会话类型", "Conversation type")); source = Choice(T("codex.source"));
        // The combined project-switcher entry still allows choosing a destination.
        // Provider-specific commands, like WebView, always target the open project.
        if (chooseDestination)
        {
            destination.Header = AutomationProperties.GetName(destination); provider.Header = AutomationProperties.GetName(provider);
            foreach (var project in projects.Where(p => !p.Id.StartsWith("assistant:", StringComparison.Ordinal)))
            {
                var item = new ComboBoxItem { Content = project.Name, Tag = project.Id }; destination.Items.Add(item);
                if (project.Id == model.Project) destination.SelectedItem = item;
            }
            provider.Items.Add(new ComboBoxItem { Content = "Codex CLI", Tag = "codex" });
            provider.Items.Add(new ComboBoxItem { Content = "Claude Code", Tag = "claude" });
            provider.SelectedIndex = initialProvider == "claude" ? 1 : 0;
            var selectors = new NativeActionWrap(); selectors.Children.Add(destination); selectors.Children.Add(provider); Notices.Children.Add(selectors);
        }
        async void Select(object sender, SelectionChangedEventArgs args)
        {
            if (rendering || model.Importing || source.SelectedItem is not ComboBoxItem context) return;
            var project = (destination.SelectedItem as ComboBoxItem)?.Tag as string ?? model.Project;
            var kind = (provider.SelectedItem as ComboBoxItem)?.Tag as string ?? initialProvider;
            model.Select(project, kind, (string)context.Tag); await model.LoadAsync(false);
        }
        destination.SelectionChanged += Select; provider.SelectionChanged += Select; source.SelectionChanged += Select;
        var toolbar = new Grid { ColumnSpacing = 12, RowSpacing = 4 };
        toolbar.ColumnDefinitions.Add(new()); toolbar.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        toolbar.RowDefinitions.Add(new() { Height = GridLength.Auto }); toolbar.RowDefinitions.Add(new() { Height = GridLength.Auto });
        var left = new StackPanel { Spacing = 4, VerticalAlignment = VerticalAlignment.Center };
        var sourceRow = new NativeActionWrap { VerticallyCentered = true, Gap = 8 };
        sourceRow.Children.Add(design.Text(T("codex.source"), 12, "text-muted")); sourceRow.Children.Add(source);
        left.Children.Add(sourceRow); hint = design.Text("", 12, "text-muted"); left.Children.Add(hint); toolbar.Children.Add(left);
        var actions = new NativeActionWrap { VerticallyCentered = true, VerticalAlignment = VerticalAlignment.Center };
        refresh = design.ToolButton(T("codex.refresh"), "sync"); refresh.Click += async (_, _) => await model.LoadAsync(true);
        importAll = Action(T("codex.import_all"), model.ImportFilteredAsync);
        var allLabel = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 };
        allLabel.Children.Add(design.Icon("download", 16)); allLabel.Children.Add(design.Text(T("codex.import_all"), 13)); importAll.Content = allLabel;
        actions.Children.Add(refresh); actions.Children.Add(importAll); Grid.SetColumn(actions, 1); toolbar.Children.Add(actions);
        toolbar.SizeChanged += (_, e) =>
        {
            var stack = e.NewSize.Width < 360;
            Grid.SetRow(actions, stack ? 1 : 0); Grid.SetColumn(actions, stack ? 0 : 1);
            Grid.SetColumnSpan(left, stack ? 2 : 1); Grid.SetColumnSpan(actions, stack ? 2 : 1);
            actions.HorizontalAlignment = stack ? HorizontalAlignment.Left : HorizontalAlignment.Right;
        };
        Notices.Children.Add(toolbar);
        query = new TextBox { PlaceholderText = T("codex.search_placeholder"), CornerRadius = new(8), MinWidth = 0, Padding = new(32, 7, 9, 7) };
        design.BindTypography(query, size: 12); AutomationProperties.SetName(query, T("codex.search"));
        PreferenceFieldAppearance.Apply(query, design);
        var search = new Grid(); search.Children.Add(query);
        var glass = design.Icon("search", 16, design.ColorText("text-faint")); glass.Margin = new(9, 0, 0, 0);
        glass.HorizontalAlignment = HorizontalAlignment.Left; glass.VerticalAlignment = VerticalAlignment.Center; glass.IsHitTestVisible = false;
        search.Children.Add(glass); Notices.Children.Add(search); query.TextChanged += (_, _) => model.SetQuery(query.Text);
        progress = design.Text("", 12, "text-muted"); AutomationProperties.SetLiveSetting(progress, Microsoft.UI.Xaml.Automation.Peers.AutomationLiveSetting.Polite);
        stop = Action(design.L("停止后续导入", "Stop after this conversation"), () => { model.StopAfterCurrent(); return Task.CompletedTask; });
        Notices.Children.Add(progress); Notices.Children.Add(progressBar); Notices.Children.Add(stop); Notices.Children.Add(errors);
        Notices.Spacing = 8; Notices.Visibility = Visibility.Visible;
        Body.Padding = new(22, 0, 22, 22); Body.Children.Add(rows);
        var pages = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 12, HorizontalAlignment = HorizontalAlignment.Center };
        previous = Action(T("codex.previous"), () => { model.SetPage(model.Page - 1); BodyScroll.ChangeView(null, 0, null, true); return Task.CompletedTask; });
        next = Action(T("codex.next"), () => { model.SetPage(model.Page + 1); BodyScroll.ChangeView(null, 0, null, true); return Task.CompletedTask; });
        pageLabel = design.Text("", 12, "text-muted"); pageLabel.VerticalAlignment = VerticalAlignment.Center;
        pages.Children.Add(previous); pages.Children.Add(pageLabel); pages.Children.Add(next); Footer.Children.Add(pages);
        Footer.Padding = new(22, 8, 22, 22);
        model.Changed += Render; Render(); _ = model.InitializeAsync();
    }
    private Button Action(string title, Func<Task> action)
    {
        var button = new Button { Content = title }; Design.ActionButton(button); Design.BindTypography(button, size: 13);
        AutomationProperties.SetName(button, title); button.Click += async (_, _) => await action(); return button;
    }
    private void Render()
    {
        if (model.Closed) return;
        rendering = true;
        try
        {
            var key = string.Join('\n', model.Sources.Select(s => s.Id + ":" + s.Label));
            if (key != sourcesKey)
            {
                sourcesKey = key; source.Items.Clear();
                foreach (var context in model.Sources)
                {
                    var option = new ComboBoxItem { Content = context.Id == "local" ? T("codex.source_local") : $"{context.Kind.ToUpperInvariant()} · {context.Label}", Tag = context.Id };
                    source.Items.Add(option); if (context.Id == model.Context) source.SelectedItem = option;
                }
            }
            hint.Text = T(ProviderKey("hint"));
            destination.IsEnabled = provider.IsEnabled = source.IsEnabled = query.IsEnabled = !model.Importing;
            refresh.IsEnabled = !model.Importing && !model.Loading; importAll.IsEnabled = model.CanImportFiltered;
            stop.Visibility = model.Importing ? Visibility.Visible : Visibility.Collapsed; stop.IsEnabled = !model.StopRequested;
            errors.Children.Clear();
            if (model.SourcesError != null)
            {
                errors.Children.Add(Warn(model.SourcesError));
                var retry = Action(Design.L("重新读取来源", "Reload sources"), model.InitializeAsync); retry.IsEnabled = !model.Importing; errors.Children.Add(retry);
            }
            if (model.Error != null) errors.Children.Add(Warn(model.Error));
            else if (model.Uncertain) errors.Children.Add(Warn(Design.L("请在目标项目核对此次导入。", "Check the unconfirmed import in the destination project.")));
            errors.Visibility = errors.Children.Count > 0 ? Visibility.Visible : Visibility.Collapsed;
            progress.Text = model.Importing ? Format("codex.progress", ("done", model.Done.ToString()), ("total", model.Total.ToString()))
                : Format(ProviderKey(model.Failed > 0 ? "summary_failed" : "summary"), ("n", (model.Imported + model.Updated).ToString()), ("f", model.Failed.ToString()));
            if (!model.Importing && model.Skipped > 0) progress.Text += " · " + Format("import.skipped", ("n", model.Skipped.ToString()));
            if (!model.Importing && model.StopRequested) progress.Text += Design.L(" · 已停止", " · Stopped");
            progress.Visibility = progressBar.Visibility = model.Total > 0 ? Visibility.Visible : Visibility.Collapsed;
            progressBar.Maximum = Math.Max(1, model.Total); progressBar.Value = model.Done; progressBar.IsIndeterminate = model.Importing && model.Done == 0;
            Footer.Visibility = model.PageCount > 1 ? Visibility.Visible : Visibility.Collapsed;
            pageLabel.Text = Format("codex.page", ("page", (model.Page + 1).ToString()), ("pages", model.PageCount.ToString()));
            previous.IsEnabled = !model.Importing && model.Page > 0; next.IsEnabled = !model.Importing && model.Page + 1 < model.PageCount;
            rows.Children.Clear();
            if (model.Loading || model.Filtered.Length == 0 && model.Error == null)
                rows.Children.Add(Mute(T(model.Loading ? "codex.loading" : !string.IsNullOrWhiteSpace(model.Query) ? "codex.no_matches" : ProviderKey("empty"))));
            foreach (var item in model.PageItems) AddRow(item);
        }
        finally { rendering = false; }
    }
    private void AddRow(ExternalImportItem item)
    {
        var line = new Grid { ColumnSpacing = 12, Padding = new(10, 8, 10, 8) };
        line.ColumnDefinitions.Add(new()); line.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var left = new StackPanel { Spacing = 5 }; line.Children.Add(left);
        var title = string.IsNullOrWhiteSpace(item.Title) ? item.SessionId : item.Title;
        var labels = new StackPanel { Spacing = 3 };
        var name = Design.Text(title, 13); name.TextWrapping = TextWrapping.NoWrap; name.TextTrimming = TextTrimming.CharacterEllipsis; labels.Children.Add(name);
        var meta = new NativeActionWrap { Gap = 10, RowGap = 2 };
        void Meta(string value) => meta.Children.Add(Design.Text(value, 11, "text-muted"));
        Meta(WorkspaceExternalImportModel.ShortPath(item.Cwd)); Meta(Format("codex.messages", ("n", item.MessageCount.ToString())));
        if (item.LastActiveAt > 0) Meta(NativeBrowserPresentation.RelativeTime(item.LastActiveAt, DateTimeOffset.UtcNow, locale: Design.Localization));
        labels.Children.Add(meta);
        var preview = new Button { Content = labels, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        Design.QuietButton(preview); preview.Padding = new(0); preview.MinHeight = 0; preview.IsEnabled = !model.Importing;
        ToolTipService.SetToolTip(preview, title + "\n" + item.Cwd);
        AutomationProperties.SetName(preview, T("codex.preview") + ": " + title);
        preview.Click += async (_, _) => { if (model.SelectedPath == item.Path) model.ClosePreview(); else await model.PreviewAsync(item); };
        left.Children.Add(preview);
        var action = Action(T(item.State == "imported" ? "codex.imported" : item.State == "updatable" ? "codex.update" : "codex.import"), () => model.ImportItemAsync(item));
        action.VerticalAlignment = VerticalAlignment.Top; action.IsEnabled = !model.Loading && !model.Previewing && !model.Importing && !model.Uncertain && item.State != "imported";
        Grid.SetColumn(action, 1); line.Children.Add(action);
        if (model.SelectedPath == item.Path)
        {
            var message = model.Previewing ? T("codex.preview_loading") : model.Preview is { } reviewed
                ? string.Join("\n\n", reviewed.Messages.Select(m => T(m.Role == "assistant" ? "codex.preview_assistant" : "codex.preview_user") + "\n" + m.Text)) : "";
            if (message.Length > 0)
            {
                var body = Design.Text(message, 11.5, "text-muted"); body.IsTextSelectionEnabled = true; Design.BindLineHeight(body, 11.5, 1.5);
                left.Children.Add(new Border { Child = body, BorderThickness = new(2, 0, 0, 0), BorderBrush = Design.Brush("clay"),
                    Background = Design.Brush("bg-sunken"), CornerRadius = new(0, 8, 8, 0), Padding = new(10, 9, 10, 9) });
            }
            var frame = model.Results.GetValueOrDefault(item.Path)?.FrameId ?? model.Preview?.ExistingSessionId;
            if (frame != null) left.Children.Add(Action(Design.L("打开已有会话", "Open conversation"), () => open(model.Project, frame)));
        }
        if (model.ItemErrors.TryGetValue(item.Path, out var error)) left.Children.Add(Warn(error));
        rows.Children.Add(line);
    }
    public override void HandleEscape()
    {
        foreach (var box in new[] { destination, provider, source }) if (box.IsDropDownOpen) { box.IsDropDownOpen = false; return; }
        base.HandleEscape();
    }
    public override void Dispose() { model.Changed -= Render; model.Dispose(); base.Dispose(); }
}
