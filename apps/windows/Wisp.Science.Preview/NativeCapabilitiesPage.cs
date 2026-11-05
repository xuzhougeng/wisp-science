using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed class NativeCapabilitiesPage : WorkspaceSheet
{
    private readonly WorkspaceActionModel runtimeRead = new(), inventoryRead = new();
    private readonly NativeLocalEnvironmentModel environment;
    private readonly StackPanel environmentRows = new() { Spacing = 6 };
    private readonly Grid tiles = new() { ColumnSpacing = 10, RowSpacing = 10 };
    private readonly WorkspaceConversationModel? conversation;
    private readonly string projectId;
    private readonly Button start;
    private int columns;

    public NativeCapabilitiesPage(INativeSettingsClient host, string project, WispDesign design, Action<string> settings,
        WorkspaceConversationModel? conversation, Action setup, Action close)
        : base(design, design.T("caps.title"), close, modal: true, modalWidth: 560, scrollWholeModal: true)
    {
        this.conversation = conversation; projectId = project;
        environment = new(host, project); environment.Changed += RenderEnvironment;
        var runtime = new StackPanel { Spacing = 8 };
        Body.Children.Add(runtime); Body.Children.Add(tiles);
        var env = new StackPanel { Spacing = 10 };
        var envTitle = Design.Text(Design.T("env.title"), 16); envTitle.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        env.Children.Add(envTitle); env.Children.Add(environmentRows);
        Body.Children.Add(new Border { BorderBrush = design.Brush("border"), BorderThickness = new(1), CornerRadius = new(10),
            Padding = new Thickness(16, 12, 16, 12), Margin = new Thickness(0, 10, 0, 0), Child = env });
        var actions = new NativeActionWrap { HorizontalAlignment = HorizontalAlignment.Right };
        actions.Children.Add(Action("caps.close", close)); start = Action("caps.setup_env", setup, true);
        if (conversation != null) conversation.Changed += UpdateSetup;
        UpdateSetup(); actions.Children.Add(start); Footer.Children.Add(actions); Footer.Visibility = Visibility.Visible;
        tiles.SizeChanged += (_, e) => LayoutTiles(e.NewSize.Width);
        Loaded += async (_, _) =>
        {
            if (!runtimeRead.Closed && !started) { started = true; await Load(); }
        };
        RenderEnvironment();

        async Task Load()
        {
            Notices.Children.Clear(); Notices.Visibility = Visibility.Collapsed;
            var readRuntime = runtime.Children.Count > 0 ? Task.CompletedTask : runtimeRead.RunAsync(async () =>
                await host.InvokeAsync("get_bootstrap_status", new(), project) as JsonObject
                    ?? throw new InvalidDataException("Missing runtime status."), snapshot =>
            {
                environment.AcceptSnapshot(snapshot); RenderEnvironment();
                runtime.Children.Clear();
                string F(string key, params (string, string)[] fields) => Design.Localization.Format(key, fields);
                var version = Design.Text(F("caps.runtime", ("version", snapshot["app_version"]?.ToString() ?? "—")), 13, "text-muted");
                version.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; runtime.Children.Add(version);
                void Hint(string text)
                {
                    var hint = Design.Text(text, 12, "text-muted"); hint.Margin = new Thickness(0, 12, 0, 12);
                    runtime.Children.Add(hint);
                }
                Hint(F("caps.workspace", ("path", snapshot["workspace"]?.ToString() ?? "—")));
                Hint(F("caps.runtime_status", ("skills", snapshot["skills_loaded"]?.ToString() ?? "—"),
                    ("mcp", snapshot["mcp_catalog"]?.ToString() ?? "—")));
                foreach (var error in snapshot["errors"] as JsonArray ?? []) if (error != null) runtime.Children.Add(Warn(error.ToString()));
            }, ex => ex.Message);
            var readInventory = tiles.Children.Count > 0 ? Task.CompletedTask : inventoryRead.RunAsync(async () =>
                NativeCapabilityTile.Read(await host.InvokeAsync("get_capabilities", new(), project), project), rows =>
            {
                tiles.Children.Clear();
                foreach (var tile in rows)
                {
                    var label = Design.T(tile.Label);
                    var content = new StackPanel { Spacing = 2, HorizontalAlignment = HorizontalAlignment.Center };
                    var number = Design.Text(tile.Count.ToString(), 22, "clay"); number.TextAlignment = TextAlignment.Center;
                    number.FontWeight = Microsoft.UI.Text.FontWeights.Bold; content.Children.Add(number);
                    var name = Design.Text(label.ToUpperInvariant(), 11, "text-faint"); name.CharacterSpacing = 40;
                    name.TextAlignment = TextAlignment.Center; content.Children.Add(name);
                    var button = new Button { Content = content, MinWidth = 0, Padding = new(12), CornerRadius = new(8),
                        Background = Design.Brush("bg-sunken"), BorderBrush = Design.Brush("border"), HorizontalAlignment = HorizontalAlignment.Stretch,
                        VerticalAlignment = VerticalAlignment.Stretch, VerticalContentAlignment = VerticalAlignment.Top,
                        HorizontalContentAlignment = HorizontalAlignment.Center };
                    AutomationProperties.SetName(button, label + ": " + tile.Count); button.Click += (_, _) => settings(tile.Section);
                    tiles.Children.Add(button);
                }
                columns = 0; LayoutTiles(tiles.ActualWidth);
            }, ex => ex.Message.Contains("Command is not available to native settings", StringComparison.Ordinal)
                ? Design.L("当前运行的桌面后端不支持能力统计。请退出旧版桌面应用，再打开新版。", "This desktop backend predates capability counts. Quit the older desktop app and reopen this version.")
                : ex.Message);
            await Task.WhenAll(readRuntime, readInventory);
            if (!runtimeRead.Closed && (runtimeRead.Error != null || inventoryRead.Error != null))
            {
                Notices.Visibility = Visibility.Visible;
                foreach (var error in new[] { runtimeRead.Error, inventoryRead.Error }.OfType<string>()) Notices.Children.Add(Warn(error));
                var retry = new Button { Content = Design.L("重试", "Retry") }; Design.ActionButton(retry);
                retry.Click += async (_, _) => await Load(); Notices.Children.Add(retry);
            }
        }
    }
    private bool started;
    protected override double ModalHeightLimit(double viewportHeight) => Math.Min(640, viewportHeight * .8);
    private void RenderEnvironment() => PopulateLocalEnvironment(environmentRows, environment, rowSpacing: 6);
    private void UpdateSetup() => start.IsEnabled = conversation?.CanStartPromptSession(projectId) == true;
    private Button Action(string key, Action click, bool primary = false)
    {
        var button = new Button { Content = Design.T(key) }; Design.ActionButton(button, primary);
        button.Click += (_, _) => click(); return button;
    }
    private void LayoutTiles(double width)
    {
        if (tiles.Children.Count == 0 || width <= 0) return;
        var count = Math.Clamp((int)((width + 10) / 130), 1, tiles.Children.Count);
        if (columns == count) return;
        columns = count; tiles.ColumnDefinitions.Clear(); tiles.RowDefinitions.Clear();
        for (int i = 0; i < count; i++) tiles.ColumnDefinitions.Add(new());
        for (int i = 0; i < tiles.Children.Count; i++)
        {
            if (i % count == 0) tiles.RowDefinitions.Add(new() { Height = GridLength.Auto });
            Grid.SetColumn((FrameworkElement)tiles.Children[i], i % count); Grid.SetRow((FrameworkElement)tiles.Children[i], i / count);
        }
    }
    public override void Dispose()
    {
        if (conversation != null) conversation.Changed -= UpdateSetup;
        runtimeRead.Dispose(); inventoryRead.Dispose(); environment.Changed -= RenderEnvironment; environment.Dispose(); base.Dispose();
    }
}
