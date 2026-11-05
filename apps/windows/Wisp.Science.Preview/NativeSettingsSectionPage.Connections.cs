using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed partial class NativeSettingsSectionPage
{
    private string? connectionDetail;
    private readonly Dictionary<string, JsonArray> discoveredTools = [];
    private readonly HashSet<string> openTools = [];
    private readonly Dictionary<string, JsonObject> connectionTransports = [];
    private TextBlock? connectionTestStatus;
    private StackPanel? connectionForm;
    private bool connectionEditorActive;
    private TextBox? connectionNameInput;

    private void BackToConnections()
    { connectionDetail = null; BodyScroll.ChangeView(null, 0, null, true); Render(); }

    private void Connections()
    {
        Form.Spacing = 12;
        Body.Padding = new Thickness(24, 14, 34, 32);
        if (connectionDetail != null) { ConnectionDetail(); return; }
        BodyScroll.ChangeView(null, 0, null, true);
        var returningFromEditor = connectionEditorActive;
        connectionEditorActive = false;
        var bundled = NativeConnections.Bundled(model.Values["list_connectors"]);
        var custom = Rows(model.Values["list_mcp_connections"]?["connections"]).ToArray();
        var toolbar = new Grid { ColumnSpacing = 14 }; toolbar.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); toolbar.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        toolbar.Children.Add(ConnectionCount(bundled.Length + custom.Length));
        var add = Button(T("conn.add"), () => { EditConnection(NativeConnections.Create(), true); return Task.CompletedTask; }); StyleModelButton(add, 12, new Thickness(12, 7, 12, 7)); Grid.SetColumn(add, 1); toolbar.Children.Add(add); Form.Children.Add(toolbar);
        if (returningFromEditor) add.Loaded += (_, _) => DispatcherQueue.TryEnqueue(Microsoft.UI.Dispatching.DispatcherQueuePriority.Low, () =>
        {
            // Wait for the replacement list's extent. Resetting during Loaded
            // can still be undone by the removed picker's scroll anchor.
            if (model.Closed || model.Draft != null || !add.IsLoaded) return;
            BodyScroll.UpdateLayout(); add.Focus(FocusState.Programmatic);
            BodyScroll.ChangeView(null, 0, null, true);
        });
        Form.Children.Add(ModelHint(T("settings.applies_new_session")));
        Form.Children.Add(ConnectionGroup("conn.featured"));
        var list = new StackPanel(); Form.Children.Add(list);
        foreach (var row in bundled) ConnectionRow(list, row, false);
        if (custom.Length > 0) { Form.Children.Add(ConnectionGroup("conn.custom")); var customList = new StackPanel(); Form.Children.Add(customList); foreach (var row in custom) ConnectionRow(customList, row, true); }
    }
    private FrameworkElement ConnectionCount(int count) => new Border
    {
        Child = Design.Text(T("settings.nav.connections") + $" ({count})", 12),
        Background = Design.Brush("bg-input"), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(1),
        CornerRadius = new CornerRadius(8), Padding = new Thickness(12, 7, 12, 7), HorizontalAlignment = HorizontalAlignment.Left,
        VerticalAlignment = VerticalAlignment.Center
    };
    private TextBlock ConnectionGroup(string key)
    {
        var block = Design.Text(T(key).ToUpperInvariant(), 11); block.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        block.Foreground = Design.Brush("text-faint"); block.CharacterSpacing = 60; block.Margin = new Thickness(2, 16, 2, 6); return block;
    }
    private void ConnectionRow(StackPanel list, JsonObject row, bool custom)
    {
        var id = S(row, custom ? "id" : "key"); var body = new Grid { ColumnSpacing = 12, Padding = new Thickness(2, 13, 2, 13) };
        body.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); body.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var copy = new StackPanel { Spacing = 4 }; var title = new NativeActionWrap(); title.Children.Add(Design.Text(S(row, "name"), 14));
        if (custom) { title.Children.Add(ConnectionBadge(S(row["transport"], "kind"))); if (S(row["transport"], "auth") == "oauth") title.Children.Add(ConnectionBadge("OAuth")); }
        copy.Children.Add(title); copy.Children.Add(ModelHint(custom ? S(row["transport"], S(row["transport"], "kind") == "http" ? "url" : "command") : Design.Localization.Format("conn.tools_count", ("n", Rows(row["tools"]).Count().ToString()))));
        if (custom) copy.Children.Add(Mute(T(B(row, "enabled") ? "conn.status.enabled" : "conn.status.disabled")));
        var open = new Button { Content = copy, HorizontalContentAlignment = HorizontalAlignment.Stretch, HorizontalAlignment = HorizontalAlignment.Stretch }; Design.QuietButton(open); open.Padding = new Thickness(0); open.MinHeight = 0; AutomationProperties.SetName(open, L("查看连接 ", "View connection ") + S(row, "name"));
        open.Click += async (_, _) => await OpenConnection(id, custom ? row : null); body.Children.Add(open);
        var actions = new NativeActionWrap { Gap = 4, VerticalAlignment = VerticalAlignment.Center };
        if (custom)
        {
            var edit = Design.ToolButton(T("conn.edit") + " " + S(row, "name"), "edit"); edit.Click += (_, _) => EditConnection(row, false); actions.Children.Add(edit);
            var remove = Design.ToolButton(T("conn.secret_remove") + " " + S(row, "name"), "close"); remove.Click += (_, _) => Confirm(L("移除连接「", "Remove connection “") + S(row, "name") + L("」？", "”?"), () => _ = Run("delete_mcp_connection", new() { ["id"] = id })); actions.Children.Add(remove);
        }
        actions.Children.Add(ConnectionToggle(T("conn.enabled_toggle") + " " + S(row, "name"), B(row, "enabled"), custom ? "set_mcp_connection_enabled" : "set_connector_enabled", new() { [custom ? "id" : "key"] = id }));
        var next = Design.ToolButton(L("打开连接 ", "Open connection ") + S(row, "name"), "chevron-right"); next.Content = Design.Icon("chevron-right", 12); next.Padding = new Thickness(0); next.MinWidth = 10; next.MinHeight = 24; next.Click += async (_, _) => await OpenConnection(id, custom ? row : null); actions.Children.Add(next);
        Grid.SetColumn(actions, 1); body.Children.Add(actions); list.Children.Add(new Border { Child = body, BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 1, 0, 0) });
    }
    private FrameworkElement ConnectionBadge(string text) => new Border { Child = Design.Text(text, 10), Background = Design.Brush("bg-sunken"), CornerRadius = new CornerRadius(7), Padding = new Thickness(6, 2, 6, 2), VerticalAlignment = VerticalAlignment.Center };
    private ToggleSwitch ConnectionToggle(string label, bool enabled, string command, JsonObject args)
    {
        var toggle = new ToggleSwitch { IsOn = enabled, OnContent = null, OffContent = null, MinWidth = 0 };
        AutomationProperties.SetName(toggle, label); StylePreferenceToggle(toggle);
        toggle.Toggled += async (_, _) => { args["enabled"] = toggle.IsOn; await Run(command, args); }; return toggle;
    }
    private async Task OpenConnection(string id, JsonObject? custom)
    {
        connectionDetail = id; BodyScroll.ChangeView(null, 0, null, true); Render();
        if (custom != null) { await model.DiscoverConnectionToolsAsync(custom, tools => discoveredTools[id] = tools); if (!model.Closed && connectionDetail == id) Render(); }
    }
    private void ConnectionDetail()
    {
        Body.Padding = new Thickness(24, 18, 34, 32); Form.Spacing = 14;
        var row = Rows(model.Values["list_connectors"]?["connectors"]).FirstOrDefault(row => S(row, "key") == connectionDetail);
        var customRow = Rows(model.Values["list_mcp_connections"]?["connections"]).FirstOrDefault(row => S(row, "id") == connectionDetail);
        var custom = customRow != null; row ??= customRow;
        if (row == null) { connectionDetail = null; Connections(); return; }
        var id = connectionDetail!; var name = S(row, "name");
        SettingsBreadcrumb(T("settings.nav.connections"), name, BackToConnections);
        var heading = new Grid { ColumnSpacing = 20, Padding = new Thickness(0, 4, 0, 16) }; heading.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); heading.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var title = new NativeActionWrap { Gap = 12, VerticallyCentered = true }; title.Children.Add(Design.Icon("grid", 24, Design.ColorText("text-muted"))); var label = Design.Text(name, 23); label.FontWeight = Microsoft.UI.Text.FontWeights.Bold; title.Children.Add(label); title.Children.Add(ConnectionBadge(T(custom ? "conn.custom_badge" : "conn.bundled_badge"))); heading.Children.Add(title);
        void ResizeTitle() => Design.BindTypography(label, NativeSettingsLayout.ConnectionCompact(XamlRoot?.Size.Width ?? ActualWidth) ? 20 : 23);
        heading.Loaded += (_, _) => ResizeTitle(); heading.SizeChanged += (_, _) => ResizeTitle();
        var enabled = ConnectionToggle(T("conn.enabled_toggle"), B(row, "enabled"), custom ? "set_mcp_connection_enabled" : "set_connector_enabled", new() { [custom ? "id" : "key"] = id }); Grid.SetColumn(enabled, 1); heading.Children.Add(enabled); Form.Children.Add(heading);
        var description = NativeConnections.Description(row, Design.Localization.Locale);
        if (description.Length > 0) { var introduction = Design.Text(description, 14); introduction.Foreground = Design.Brush("text-muted"); introduction.LineHeight = Design.FontSize(23.8); introduction.MaxWidth = Design.FontSize(810); introduction.HorizontalAlignment = HorizontalAlignment.Left; introduction.Margin = new Thickness(0, 0, 0, 16); Form.Children.Add(introduction); }
        Form.Children.Add(ModelHint(T("settings.applies_new_session")));
        if (custom)
        {
            var info = new StackPanel { Spacing = 14 };
            info.Children.Add(Mute(T("conn.service"))); info.Children.Add(Design.Text(S(customRow!["transport"], S(customRow["transport"], "kind") == "http" ? "url" : "command"), 14));
            info.Children.Add(Mute(T("conn.status"))); info.Children.Add(Design.Text(T(B(row, "enabled") ? "conn.status.enabled" : "conn.status.disabled"), 14));
            if (S(customRow["transport"], "kind") == "http") { info.Children.Add(Mute(T("conn.auth"))); info.Children.Add(Design.Text(T(S(customRow["transport"], "auth") == "oauth" ? "conn.auth.oauth" : "conn.auth.none"), 14)); } Form.Children.Add(info);
        }
        else
        {
            var approvals = new StackPanel(); InlineRow(approvals, T("conn.skip_approvals"), T("conn.skip_approvals.desc"), ConnectionToggle(T("conn.skip_approvals"), B(row, "skip_approvals"), "set_connector_skip_approvals", new() { ["key"] = id }), compact: true, divider: false);
            Form.Children.Add(new Border { Child = approvals, BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 1, 0, 0) });
        }
        Form.Children.Add(ConnectionGroup("conn.tools"));
        var toolsHint = ModelHint(T("conn.tools_hint")); toolsHint.Margin = new Thickness(0, -2, 0, 12); Form.Children.Add(toolsHint);
        var tools = custom ? discoveredTools.GetValueOrDefault(id) : row["tools"] as JsonArray;
        if (custom && tools == null) Form.Children.Add(Mute(model.Busy ? T("conn.tools_loading") : model.Error != null ? Design.Localization.Format("conn.tools_failed", ("msg", model.Error)) : T("conn.no_tools")));
        else if (tools?.Count == 0) Form.Children.Add(Mute(T("conn.no_tools")));
        var list = new StackPanel(); Form.Children.Add(list);
        foreach (var tool in Rows(tools)) ConnectorTool(list, id, tool, custom, B(row, "skip_approvals"));
        if (list.Children.LastOrDefault() is Border lastTool) lastTool.BorderThickness = new Thickness(0);
        if (S(row, "maintainer").Length > 0 || Rows(row["links"]).Any())
        {
            var sources = new StackPanel(); var sourceTitle = Design.Text(T("conn.details"), 15); sourceTitle.FontWeight = Microsoft.UI.Text.FontWeights.Bold; sourceTitle.Margin = new Thickness(0, 0, 0, 18); sources.Children.Add(sourceTitle);
            if (S(row, "maintainer").Length > 0) { sources.Children.Add(ConnectionCaption(T("conn.maintainer"))); var maintainer = Design.Text(S(row, "maintainer"), 13); maintainer.Margin = new Thickness(0, 0, 0, 16); sources.Children.Add(maintainer); }
            sources.Children.Add(ConnectionCaption(T("conn.sources")));
            var links = new NativeActionWrap { Gap = 22, RowGap = 10, Margin = new Thickness(0, 4, 0, 0) };
            foreach (var link in Rows(row["links"])) if (Uri.TryCreate(S(link, "url"), UriKind.Absolute, out var uri) && uri.Scheme is "http" or "https")
            { var linkCopy = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 }; var linkText = Design.Text(S(link, "label"), 13); linkText.Foreground = Design.Brush("clay"); linkCopy.Children.Add(linkText); linkCopy.Children.Add(Design.Icon("external-link", 14, Design.ColorText("clay"))); var button = new HyperlinkButton { Content = linkCopy, NavigateUri = uri, HorizontalAlignment = HorizontalAlignment.Left, Padding = new Thickness(0) }; AutomationProperties.SetName(button, S(link, "label")); ToolTipService.SetToolTip(button, uri.AbsoluteUri); links.Children.Add(button); }
            sources.Children.Add(links); Form.Children.Add(new Border { Child = sources, Margin = new Thickness(0, 24, 0, 0), Padding = new Thickness(0, 20, 0, 0), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 1, 0, 0) });
        }
    }
    private void ConnectorTool(StackPanel list, string connector, JsonObject tool, bool custom, bool skip)
    {
        var name = S(tool, "name"); var key = connector + "\n" + name;
        var group = new StackPanel { Spacing = 0 };
        var line = new Grid { ColumnSpacing = 16 }; line.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); line.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var documentation = ToolDocumentation(tool); documentation.Visibility = openTools.Contains(key) ? Visibility.Visible : Visibility.Collapsed;
        var copy = new Grid { ColumnSpacing = 10, HorizontalAlignment = HorizontalAlignment.Stretch }; copy.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); copy.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); copy.Children.Add(Design.Icon(openTools.Contains(key) ? "chevron-down" : "chevron-right", 16, Design.ColorText("text-faint"))); var toolName = Design.Text(name, 13); toolName.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; Grid.SetColumn(toolName, 1); copy.Children.Add(toolName);
        var disclose = new Button { Content = copy, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch }; Design.QuietButton(disclose); disclose.MinHeight = 58; disclose.Padding = new Thickness(0, 14, 0, 14); disclose.FocusVisualPrimaryBrush = Design.Brush("clay"); AutomationProperties.SetName(disclose, T("conn.tools_hint") + " " + name);
        disclose.Click += (_, _) => { var expanded = openTools.Add(key); if (!expanded) openTools.Remove(key); documentation.Visibility = expanded ? Visibility.Visible : Visibility.Collapsed; copy.Children[0] = Design.Icon(expanded ? "chevron-down" : "chevron-right", 16, Design.ColorText("text-faint")); }; line.Children.Add(disclose);
        void ResizeTool() => line.ColumnSpacing = NativeSettingsLayout.ConnectionCompact(XamlRoot?.Size.Width ?? ActualWidth) ? 8 : 16;
        line.Loaded += (_, _) => ResizeTool(); line.SizeChanged += (_, _) => ResizeTool();
        if (!custom)
        {
            var segment = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 0, VerticalAlignment = VerticalAlignment.Center };
            foreach (var (mode, icon, label, color) in new[] { ("allow", "check", T("conn.approval.allow"), "ok"), ("ask", "hand", T("conn.approval.ask"), "warn"), ("deny", "ban", T("conn.approval.deny"), "err") })
            {
                var active = S(tool, "mode") == mode || S(tool, "mode").Length == 0 && mode == "allow";
                var button = new Button { Content = Design.Icon(icon, 14, active ? "#ffffff" : null), Padding = new Thickness(8, 6, 8, 6), MinWidth = 30, MinHeight = 28, CornerRadius = mode == "allow" ? new CornerRadius(8, 0, 0, 8) : mode == "deny" ? new CornerRadius(0, 8, 8, 0) : new CornerRadius(0), IsEnabled = !skip, Background = Design.Brush(active ? color : "bg-elev"), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(1) };
                AutomationProperties.SetName(button, label + " " + name); ToolTipService.SetToolTip(button, label); button.Click += async (_, _) => await Run("set_tool_approval", new() { ["tool"] = name, ["mode"] = mode }); segment.Children.Add(button);
            }
            Grid.SetColumn(segment, 1); line.Children.Add(segment);
        }
        group.Children.Add(line); group.Children.Add(documentation); list.Children.Add(new Border { Child = group, BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 0, 0, 1) });
    }
    private FrameworkElement ToolDocumentation(JsonObject tool)
    {
        var body = new StackPanel { Margin = new Thickness(26, 0, 0, 20) };
        void ResizeDocumentation() => body.Margin = new Thickness(NativeSettingsLayout.ConnectionCompact(XamlRoot?.Size.Width ?? ActualWidth) ? 0 : 26, 0, 0, 20);
        body.Loaded += (_, _) => ResizeDocumentation(); body.SizeChanged += (_, _) => ResizeDocumentation();
        var introduction = Design.Text(S(tool, "description") is { Length: > 0 } description ? description : T("conn.description_missing"), 13);
        introduction.LineHeight = Design.FontSize(22.75); introduction.LineStackingStrategy = LineStackingStrategy.BlockLineHeight; introduction.MaxWidth = Design.FontSize(760); introduction.HorizontalAlignment = HorizontalAlignment.Left; introduction.Margin = new Thickness(0, 0, 0, 18); body.Children.Add(introduction);
        var schema = tool["input_schema"] ?? tool["inputSchema"];
        if (schema != null)
        {
            var caption = ConnectionCaption(T("conn.parameters")); caption.FontWeight = Microsoft.UI.Text.FontWeights.Bold; caption.Margin = new Thickness(0, 0, 0, 10); body.Children.Add(caption); var parameters = NativeConnections.Parameters(schema);
            if (parameters.Length == 0) body.Children.Add(ModelHint(T("conn.no_named_parameters")));
            else
            {
                var table = new StackPanel();
                TextBlock Cell(string value, bool code = false)
                {
                    var text = Design.Text(value, 12); if (code) text.FontFamily = Design.CodeFont;
                    return text;
                }
                void Row(FrameworkElement name, FrameworkElement kind, FrameworkElement description)
                {
                    var line = new Grid { ColumnSpacing = 14, Padding = new Thickness(0, 10, 14, 10) }; line.ColumnDefinitions.Add(new() { Width = new GridLength(1.2, GridUnitType.Star) }); line.ColumnDefinitions.Add(new() { Width = new GridLength(.8, GridUnitType.Star) }); line.ColumnDefinitions.Add(new() { Width = new GridLength(1.1, GridUnitType.Star) });
                    line.Children.Add(name); Grid.SetColumn(kind, 1); line.Children.Add(kind); Grid.SetColumn(description, 2); line.Children.Add(description);
                    table.Children.Add(new Border { Child = line, BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 0, 0, 1) });
                }
                TextBlock Header(string key) { var text = Cell(T(key)); text.Foreground = Design.Brush("text-muted"); text.FontWeight = Microsoft.UI.Text.FontWeights.Medium; return text; }
                Row(Header("conn.parameter"), Header("conn.parameter_type"), Header("conn.parameter_description"));
                foreach (var parameter in parameters)
                {
                    var name = new StackPanel { Spacing = 4 }; name.Children.Add(Cell(parameter.Name, code: true));
                    if (parameter.Required) { var required = Design.Text(T("conn.required"), 10); required.Foreground = Design.Brush("text-muted"); name.Children.Add(required); }
                    var parameterCopy = new StackPanel();
                    if (parameter.Description.Length > 0) parameterCopy.Children.Add(Cell(parameter.Description));
                    if (parameter.Default != null)
                    {
                        var value = Cell(""); value.Foreground = Design.Brush("text-muted");
                        value.Inlines.Add(new Microsoft.UI.Xaml.Documents.Run { Text = T("conn.default_value") + ": " });
                        value.Inlines.Add(new Microsoft.UI.Xaml.Documents.Run { Text = parameter.Default, FontFamily = Design.CodeFont }); parameterCopy.Children.Add(value);
                    }
                    Row(name, Cell(parameter.Type, code: true), parameterCopy);
                }
                body.Children.Add(table);
            }
            body.Children.Add(ConnectionSchema(T("conn.input_schema"), schema));
            if ((schema as JsonObject)?["examples"] is JsonArray { Count: > 0 } examples) { var examplesCaption = ConnectionCaption(T("conn.examples")); examplesCaption.FontWeight = Microsoft.UI.Text.FontWeights.Bold; examplesCaption.Margin = new Thickness(0, 16, 0, 10); body.Children.Add(examplesCaption); body.Children.Add(JsonText(examples)); }
        }
        if ((tool["output_schema"] ?? tool["outputSchema"]) is { } output) body.Children.Add(ConnectionSchema(T("conn.output_schema"), output));
        return body;
    }
    private TextBlock ConnectionCaption(string title)
    {
        var caption = Design.Text(title, 12); caption.Foreground = Design.Brush("text-muted"); caption.FontWeight = Microsoft.UI.Text.FontWeights.Medium; caption.Margin = new Thickness(0, 0, 0, 6); return caption;
    }
    private FrameworkElement ConnectionSchema(string title, JsonNode schema)
    {
        var body = new StackPanel { Spacing = 8, Margin = new Thickness(0, 14, 0, 0) };
        var text = JsonText(schema); text.Visibility = Visibility.Collapsed;
        var copy = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4 };
        copy.Children.Add(Design.Icon("chevron-right", 12)); copy.Children.Add(ModelHint(title));
        var button = new Button { Content = copy, HorizontalAlignment = HorizontalAlignment.Left };
        Design.QuietButton(button); button.MinHeight = 0; button.Padding = new Thickness(0, 4, 0, 4); AutomationProperties.SetName(button, title);
        button.Click += (_, _) => { var expanded = text.Visibility != Visibility.Visible; text.Visibility = expanded ? Visibility.Visible : Visibility.Collapsed; copy.Children[0] = Design.Icon(expanded ? "chevron-down" : "chevron-right", 12); };
        body.Children.Add(button); body.Children.Add(text); return body;
    }
    private TextBox JsonText(JsonNode value)
        { var text = new TextBox { IsReadOnly = true, AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, Text = NativeConnections.Json(value), MaxHeight = 360, Padding = new Thickness(12), CornerRadius = new CornerRadius(6), BorderThickness = new Thickness(1), BorderBrush = Design.Brush("border"), Background = Design.Brush("bg-input") }; Design.BindTypography(text, size: 12, code: true); textMenus.Register(text); return text; }

    private void EditConnection(JsonObject conn, bool create)
    {
        connectionEditorActive = true;
        connectionTransports.Clear();
        var source = (JsonObject)conn.DeepClone(); var transport = source["transport"]!.AsObject();
        foreach (var secret in Rows(transport[S(transport, "kind") == "http" ? "headers" : "env"])) secret.Remove("value");
        Editor(T(create ? "conn.add" : "conn.edit"), source, create ? "add_mcp_connection" : "update_mcp_connection", "conn", DrawConnectionForm, styleActions: false);
        SettingsBreadcrumb(T("settings.nav.connections"), T(create ? "conn.add" : "conn.edit"), () => RequestLeave(() => { model.Discard(); Render(); }));
        var buttons = editorActions.Children.OfType<Button>().ToArray();
        buttons[0].Content = Design.Text(T("settings.cancel"), 13.5); buttons[1].Content = Design.Text(T("settings.save"), 13.5);
        AutomationProperties.SetName(buttons[0], T("settings.cancel")); AutomationProperties.SetName(buttons[1], T("settings.save"));
        foreach (var button in buttons) { StyleModelButton(button, padding: new Thickness(16, 9, 16, 9), primary: button == buttons[1]); button.MinWidth = 84; }
        ((TextBlock)buttons[1].Content).FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        AddConnectionTest();
        var editedDraft = model.Draft;
        if (connectionNameInput is { } nameInput) nameInput.Loaded += (_, _) =>
        {
            if (model.Closed || model.Draft != editedDraft || connectionNameInput != nameInput) return;
            nameInput.Focus(FocusState.Programmatic); BodyScroll.ChangeView(null, 0, null, true);
        };
    }
    private void RedrawConnectionForm()
    {
        connectionForm?.Children.Remove(editorActions);
        textMenus.CloseTop(); Form.Children.Clear(); choices.Clear(); connectionTestStatus = null;
        DrawConnectionForm(model.Draft!); Design.ApplyTypography(Form); LockConfirmation();
    }
    private void AddConnectionTest()
    {
        var test = ModelButton(T("conn.test"), async () =>
        {
            if (model.Draft is { } proxyDraft && NativeProxyRoutes.ErrorKey(proxyDraft) is { } routeError) { model.Fail(T(routeError)); return; }
            if (connectionTestStatus != null) connectionTestStatus.Text = T("conn.testing_status");
            var ok = await model.TestConnectionAsync(tools => { if (connectionTestStatus != null) connectionTestStatus.Text = L("连接成功 — ", "Connected — ") + Design.Localization.Format("conn.tools_count", ("n", tools.Count.ToString())); });
            if (!ok && connectionTestStatus != null) connectionTestStatus.Text = L("连接结果未确认，请查看错误详情。", "Connection not confirmed. See the error details.");
        });
        StyleModelButton(test, padding: new Thickness(16, 9, 16, 9)); test.MinWidth = 84; editorActions.Children.Insert(0, test); LockConfirmation();
    }
    private void DrawConnectionForm(JsonObject draft)
    {
        // Keep the owning panel: Parent can be null after its ancestor is removed,
        // even while its Children collection still owns this reusable action row.
        connectionForm?.Children.Remove(editorActions);
        if (editorActions.Parent is Panel previous) previous.Children.Remove(editorActions);
        Body.Padding = new Thickness(24, 20, 24, 32);
        var form = new StackPanel { Spacing = 10 }; connectionForm = form;
        var frame = new Border { Child = form, Padding = new Thickness(16, 14, 16, 14), CornerRadius = new CornerRadius(8), Background = Design.Brush("bg-sunken"), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(1) }; Form.Children.Add(frame);
        connectionNameInput = ConnectionInput(form, T("conn.name"), S(draft, "name"), value => draft["name"] = value);
        var transport = draft["transport"]!.AsObject(); var kind = S(transport, "kind");
        var type = ConnectionChoice(T("conn.kind"));
        foreach (var (id, label) in new[] { ("stdio", T("conn.kind.stdio")), ("http", T("conn.kind.http")) }) type.Items.Add(new ComboBoxItem { Content = label, Tag = id });
        type.SelectedItem = type.Items.Cast<ComboBoxItem>().First(row => (string)row.Tag == kind); choices.Add(type); ConnectionField(form, type, T("conn.kind"));
        type.SelectionChanged += (_, _) => { if (type.SelectedItem is ComboBoxItem item && (string)item.Tag != kind) { connectionTransports[kind] = (JsonObject)transport.DeepClone(); draft["transport"] = connectionTransports.TryGetValue((string)item.Tag, out var cached) ? cached.DeepClone() : NativeConnections.Transport((string)item.Tag); RedrawConnectionForm(); } };
        if (kind == "stdio")
        {
            ConnectionInput(form, T("conn.command"), S(transport, "command"), value => transport["command"] = value);
            ConnectionInput(form, T("conn.args"), string.Join(" ", (transport["args"] as JsonArray ?? []).Select(value => value?.GetValue<string>() ?? "")), value => transport["args"] = new JsonArray(value.Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries).Select(value => (JsonNode?)JsonValue.Create(value)).ToArray()), "arg1 arg2");
            ConnectionSecrets(form, transport, "env", T("conn.env"));
            // Existing per-server working directories are retained even though WebView does not expose an editor.
            if (S(transport, "cwd").Length > 0) form.Children.Add(Mute(L("工作目录：", "Working directory: ") + S(transport, "cwd")));
        }
        else
        {
            ConnectionInput(form, T("conn.url"), S(transport, "url"), value => transport["url"] = value, "https://host/mcp");
            var auth = ConnectionChoice(T("conn.auth"));
            foreach (var (id, label) in new[] { ("none", T("conn.auth.none")), ("oauth", T("conn.auth.oauth")) }) auth.Items.Add(new ComboBoxItem { Content = label, Tag = id });
            auth.SelectedItem = auth.Items.Cast<ComboBoxItem>().FirstOrDefault(item => (string)item.Tag == S(transport, "auth")); choices.Add(auth); ConnectionField(form, auth, T("conn.auth"));
            auth.SelectionChanged += (_, _) => { if (auth.SelectedItem is ComboBoxItem item) { transport["auth"] = (string)item.Tag; RedrawConnectionForm(); } };
            ConnectionSecrets(form, transport, "headers", T("conn.headers"));
            if (S(transport, "auth") == "oauth") form.Children.Add(ModelHint(T("conn.oauth.desc")));
        }
        form.Children.Add(ProxyRouteField(() => S(draft, "proxy_url"), () => NativeProxyRoutes.Custom(draft),
            (value, custom) => NativeProxyRoutes.Set(draft, value, custom), "network.mcp", "conn.proxy_hint", out _));
        var feedback = ModelHint(""); connectionTestStatus = feedback; feedback.Visibility = Visibility.Collapsed;
        feedback.RegisterPropertyChangedCallback(TextBlock.TextProperty, (_, _) => feedback.Visibility = string.IsNullOrEmpty(feedback.Text) ? Visibility.Collapsed : Visibility.Visible);
        form.Children.Add(connectionTestStatus);
        editorActions.Margin = new Thickness(0, 24, 0, 0);
        form.Children.Add(editorActions);
    }
    private ComboBox ConnectionChoice(string label)
    {
        var choice = new ComboBox { Header = ModelHint(label), HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch, MinHeight = 36, CornerRadius = new CornerRadius(8), Background = Design.Brush("bg-input"), BorderBrush = Design.Brush("border-strong"), BorderThickness = new Thickness(1), Padding = new Thickness(10, 8, 0, 8) };
        Design.BindTypography(choice, size: 13); AutomationProperties.SetName(choice, label); return choice;
    }
    private TextBox ConnectionInput(StackPanel target, string label, string value, Action<string> changed, string placeholder = "")
    {
        var input = new TextBox { Text = value, PlaceholderText = placeholder, MinHeight = 36, CornerRadius = new CornerRadius(8) };
        AutomationProperties.SetName(input, label.Length > 0 ? label : placeholder); textMenus.Register(input); input.TextChanged += (_, _) => { changed(input.Text); LockConfirmation(); }; ConnectionField(target, input, label); return input;
    }
    private void ConnectionField(StackPanel target, Control input, string label)
    {
        if (input is ComboBox choice) choice.Header = null;
        Design.BindTypography(input, size: 14); input.Padding = new Thickness(11, 9, input is ComboBox ? 0 : 11, 9);
        PreferenceFieldAppearance.Apply(input, Design);
        if (label.Length == 0) { target.Children.Add(input); return; }
        var caption = Design.Text(label, 12); caption.Foreground = Design.Brush("text-muted"); caption.FontWeight = Microsoft.UI.Text.FontWeights.Medium;
        var group = new StackPanel { Spacing = 5 }; group.Children.Add(caption); group.Children.Add(input); target.Children.Add(group);
        AutomationProperties.SetLabeledBy(input, caption);
    }
    private void ConnectionSecrets(StackPanel target, JsonObject transport, string key, string title)
    {
        if (transport[key] is not JsonArray rows) { rows = new JsonArray(); transport[key] = rows; }
        var group = new StackPanel { Spacing = 6 }; target.Children.Add(group);
        var caption = Design.Text(title, 12); caption.Foreground = Design.Brush("text-muted"); group.Children.Add(caption);
        var list = new StackPanel { Spacing = 6 }; group.Children.Add(list);
        void Add(JsonObject row)
        {
            var line = new Grid { ColumnSpacing = 6, RowSpacing = 6 }; line.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); line.ColumnDefinitions.Add(new() { Width = new GridLength(1.2, GridUnitType.Star) }); line.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            var hints = NativeConnections.SecretPlaceholders(transport, row, Design.Localization);
            var nameHost = new StackPanel(); ConnectionInput(nameHost, "", S(row, "name"), value => row["name"] = value, hints.Name); line.Children.Add(nameHost);
            var valueHost = new Grid(); Grid.SetColumn(valueHost, 1); line.Children.Add(valueHost);
            var value = new PasswordBox { Password = S(row, "value"), PlaceholderText = hints.Value, PasswordRevealMode = PasswordRevealMode.Hidden, MinHeight = 36, CornerRadius = new CornerRadius(8), Padding = new Thickness(11, 9, 36, 9) };
            Design.BindTypography(value, size: 14); PreferenceFieldAppearance.Apply(value, Design);
            AutomationProperties.SetName(value, title + " " + T("conn.secret_value"));
            value.PasswordChanged += (_, _) => { row["value"] = value.Password; LockConfirmation(); };
            valueHost.Children.Add(value);
            var reveal = new Microsoft.UI.Xaml.Controls.Primitives.ToggleButton
            {
                Width = 28, Height = 28, MinWidth = 0, MinHeight = 0, Padding = new Thickness(0),
                Margin = new Thickness(0, 0, 4, 0), HorizontalAlignment = HorizontalAlignment.Right,
                VerticalAlignment = VerticalAlignment.Center, CornerRadius = new CornerRadius(8),
                BorderThickness = new Thickness(0), Background = new Microsoft.UI.Xaml.Media.SolidColorBrush(Microsoft.UI.Colors.Transparent)
            };
            foreach (var state in new[] { "PointerOver", "Pressed", "CheckedPointerOver", "CheckedPressed" })
                reveal.Resources["ToggleButtonBackground" + state] = Design.Brush("bg-sunken");
            reveal.Resources["ToggleButtonBackgroundChecked"] = reveal.Background;
            void UpdateReveal(bool focus)
            {
                var visible = reveal.IsChecked == true;
                value.PasswordRevealMode = visible ? PasswordRevealMode.Visible : PasswordRevealMode.Hidden;
                reveal.Content = Design.Icon(visible ? "eye-off" : "eye", 14, Design.ColorText("text-faint"));
                var label = T(visible ? "conn.secret_hide" : "conn.secret_show");
                AutomationProperties.SetName(reveal, label); ToolTipService.SetToolTip(reveal, label);
                if (focus) value.Focus(FocusState.Programmatic);
            }
            reveal.Checked += (_, _) => UpdateReveal(true); reveal.Unchecked += (_, _) => UpdateReveal(true);
            UpdateReveal(false); valueHost.Children.Add(reveal);
            var remove = Design.ToolButton(T("conn.secret_remove") + " " + title, "close"); remove.Click += (_, _) => { rows.Remove(row); list.Children.Remove(line); LockConfirmation(); }; Grid.SetColumn(remove, 2); line.Children.Add(remove); list.Children.Add(line);
        }
        foreach (var row in Rows(rows)) Add(row);
        var add = Design.ToolButton(T(key == "env" ? "conn.secret_add_env" : "conn.secret_add_header"), "plus", true); StyleModelButton(add); add.HorizontalAlignment = HorizontalAlignment.Left; add.Click += (_, _) => { var row = new JsonObject { ["name"] = "", ["value"] = "" }; rows.Add(row); Add(row); LockConfirmation(); }; group.Children.Add(add);
        var hint = ModelHint(T("conn.secret_hint")); hint.Margin = new Thickness(0, 12, 0, 14); group.Children.Add(hint);
    }
}
