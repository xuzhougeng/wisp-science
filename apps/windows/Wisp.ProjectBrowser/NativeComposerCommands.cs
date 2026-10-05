namespace Wisp.ProjectBrowser;

public sealed record NativeComposerCommand(string Command, string Label)
{
    public override string ToString() => $"{Command}   {Label}";
}

public static class NativeComposerCommands
{
    public static IReadOnlyList<NativeComposerCommand> All { get; } =
    [
        new("/upload", "添加附件"), new("/files", "项目文件"), new("/outline", "会话大纲"),
        new("/share", "分享会话"), new("/trajectory", "运行轨迹"), new("/archive", "研究归档"),
        new("/library", "收藏库"), new("/calendar", "研究日历"), new("/journey", "研究历程"),
        new("/publication", "论文证据"), new("/settings", "设置")
    ];

    public static IReadOnlyList<NativeComposerCommand> Match(string draft, bool canUpload, bool canNavigate)
    {
        var query = draft.TrimStart();
        if (!query.StartsWith('/') || query.Any(char.IsWhiteSpace)) return [];
        return All.Where(item => (item.Command == "/upload" ? canUpload : canNavigate)
            && item.Command.StartsWith(query, StringComparison.OrdinalIgnoreCase)).ToArray();
    }

    public static NativeComposerCommand? Exact(string draft) => All.FirstOrDefault(item =>
        item.Command.Equals(draft.Trim(), StringComparison.OrdinalIgnoreCase));

    public static bool ShouldSubmit(bool control, bool shift, bool composing, bool sendWithModifier = true)
        => (control || !sendWithModifier) && !shift && !composing;
}
