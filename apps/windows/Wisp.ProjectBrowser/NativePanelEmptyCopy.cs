namespace Wisp.ProjectBrowser;

/// Empty panels use the same catalog entries as the WebView, including filtered results.
public sealed record NativePanelEmptyCopy(string Title, string Body)
{
    public static NativePanelEmptyCopy For(string panel, NativeLocalization locale, bool filtered = false)
    {
        if (filtered) return new(locale.Choose("没有匹配的记录", "No matching records"),
            locale.Choose("尝试其他关键词，或清空筛选查看全部记录。", "Try another search, or clear the filter to see all records."));
        var key = panel switch
        {
            "files" => "file", "notebook" => "notebook", "highlights" => "highlights",
            "provenance" => "tools", "runs" => "runs", _ => "artifacts"
        };
        return new(locale.Text("right.no_" + key + ".title"), locale.Text("right.no_" + key + ".body"));
    }
}
