using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed class NativeSessionArchiveImportPage : WorkspaceSheet
{
    private readonly WorkspaceArchiveImportModel model;
    private readonly ComboBox destination;
    private readonly TextBox path;
    private readonly Button pick, preview, import;
    private readonly StackPanel result = new() { Spacing = 10 };

    public NativeSessionArchiveImportPage(WorkspaceArchiveImportModel model, IReadOnlyList<ProjectSummary> projects,
        WispDesign design, Func<Task<string?>> pickArchive, Func<string, string, Task> open, Action close)
        : base(design, "导入会话归档", close)
    {
        this.model = model;
        Body.Children.Add(Mute("选择 Wisp 导出的会话 ZIP，预览内容后导入指定项目。预览不会创建会话。"));
        destination = new ComboBox { Header = "目标项目", HorizontalAlignment = HorizontalAlignment.Stretch };
        AutomationProperties.SetName(destination, "目标项目"); design.BindTypography(destination);
        foreach (var project in projects.Where(p => !p.Id.StartsWith("assistant:", StringComparison.Ordinal)))
        {
            var item = new ComboBoxItem { Content = project.Name, Tag = project.Id };
            destination.Items.Add(item); if (project.Id == model.Project) destination.SelectedItem = item;
        }
        Body.Children.Add(destination);
        path = new TextBox { Header = "会话归档路径", Text = model.Path, PlaceholderText = "选择会话 ZIP 文件" };
        AutomationProperties.SetName(path, "会话归档路径"); design.BindTypography(path); Body.Children.Add(path);
        destination.SelectionChanged += (_, _) => { if (destination.SelectedItem is ComboBoxItem item) model.Select((string)item.Tag, path.Text); };
        path.TextChanged += (_, _) => model.Select(model.Project, path.Text);
        Button Action(string title, Func<Task> action)
        {
            var button = new Button { Content = title }; Design.ActionButton(button);
            AutomationProperties.SetName(button, title); button.Click += async (_, _) => await action(); return button;
        }
        var actions = new NativeActionWrap();
        pick = Action("选择会话归档", async () => { if (await pickArchive() is { } selected && !model.Closed) path.Text = selected; });
        preview = Action("预览归档", model.PreviewAsync);
        import = Action("确认导入到所选项目", model.ImportAsync);
        actions.Children.Add(pick); actions.Children.Add(preview); actions.Children.Add(import); Body.Children.Add(actions);
        Body.Children.Add(result);
        void Render()
        {
            if (model.Closed) return;
            destination.IsEnabled = path.IsEnabled = pick.IsEnabled = !model.Importing;
            preview.IsEnabled = !model.Importing && !model.Reading;
            import.IsEnabled = model.CanImport;
            Notices.Children.Clear();
            if (model.Reading || model.Importing) Notices.Children.Add(Mute(model.Importing ? "正在导入，请勿重复提交…" : "正在读取归档预览…"));
            if (model.Error != null) Notices.Children.Add(Warn(model.Error));
            else if (model.Uncertain) Notices.Children.Add(Warn("此前的导入未能确认。本窗口仅供核对已有会话，不会再次提交；请核对后关闭。"));
            Notices.Visibility = Notices.Children.Count == 0 ? Visibility.Collapsed : Visibility.Visible;
            result.Children.Clear();
            if (model.Preview is { } reviewed)
            {
                result.Children.Add(design.Text(reviewed.Title, 18));
                var state = reviewed.State switch { "imported" => "已导入，当前归档不会增加消息", "updatable" => "可更新已有导入会话", _ => "将创建新会话" };
                result.Children.Add(Mute($"{reviewed.MessageCount} 条消息 · {reviewed.Artifacts.Length} 个产物" + (model.Result == null ? $" · {state}" : " · 已确认的归档内容")));
                result.Children.Add(Mute("前 4 条用户／助手消息预览（每条最多 600 字符）"));
                foreach (var message in reviewed.Messages)
                {
                    result.Children.Add(design.Text(message.Role == "user" ? "你" : "助手", 13));
                    var text = new TextBox { IsReadOnly = true, AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, Text = message.Text, MaxHeight = 160 };
                    design.BindTypography(text); result.Children.Add(text);
                }
                if (reviewed.Artifacts.Length > 0)
                {
                    result.Children.Add(design.Text("归档中的产物", 14));
                    var files = new ListView { ItemsSource = reviewed.Artifacts, MaxHeight = 160, SelectionMode = ListViewSelectionMode.None };
                    AutomationProperties.SetName(files, "归档产物列表"); result.Children.Add(files);
                }
                if (model.Uncertain && reviewed.ExistingSessionId is { } existing)
                    result.Children.Add(Action("查看目标项目中的已有会话", () => open(reviewed.ProjectId, existing)));
            }
            if (model.Result is { } imported)
            {
                var status = imported.Status switch { "updated" => "已更新", "skipped" => "已存在，无需重复导入", _ => "导入完成" };
                result.Children.Insert(0, design.Text($"{status} · {imported.MessageCount} 条消息 · 已恢复 {imported.ArtifactCount} 个产物", 16));
                if (imported.MissingArtifacts.Length > 0) result.Children.Add(Warn("部分产物未恢复：\n" + string.Join("\n", imported.MissingArtifacts)));
                result.Children.Insert(1, Action("打开导入的会话", () => open(imported.ProjectId, imported.FrameId)));
            }
            Design.ApplyTypography(result);
        }
        render = Render; model.Changed += render; Render();
    }
    private readonly Action render;
    public override void HandleEscape()
    {
        if (destination.IsDropDownOpen) { destination.IsDropDownOpen = false; return; }
        base.HandleEscape();
    }
    public override void Dispose() { model.Changed -= render; model.Dispose(); base.Dispose(); }
}
