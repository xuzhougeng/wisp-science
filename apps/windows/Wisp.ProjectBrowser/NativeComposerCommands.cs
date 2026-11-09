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
        new("/publication", "论文整理"), new("/settings", "设置"), new("/compact", "压缩上下文")
    ];

    public static IReadOnlyList<NativeComposerCommand> Match(string draft, bool canUpload, bool canNavigate, bool canCompact = false)
    {
        var query = draft.TrimStart();
        if (!query.StartsWith('/') || query.Any(char.IsWhiteSpace)) return [];
        return All.Where(item => (item.Command == "/upload" ? canUpload : item.Command == "/compact" ? canCompact : canNavigate)
            && item.Command.StartsWith(query, StringComparison.OrdinalIgnoreCase)).ToArray();
    }

    public static NativeComposerCommand? Exact(string draft) => All.FirstOrDefault(item =>
        item.Command.Equals(draft.Trim(), StringComparison.OrdinalIgnoreCase));

    public static bool TryCompaction(string draft, out string instruction)
    {
        var text = draft.Trim(); instruction = "";
        if (!text.StartsWith("/compact", StringComparison.OrdinalIgnoreCase)
            || text.Length > 8 && !char.IsWhiteSpace(text[8])) return false;
        instruction = text[8..].Trim(); return true;
    }

    public static bool ShouldSubmit(bool control, bool shift, bool composing, bool sendWithModifier = true)
        => (control || !sendWithModifier) && !shift && !composing;
}
