using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed record NativeToolSection(string Label, string Text);
public static class NativeStructuredTool
{
    public static NativeToolSection[] Sections(ConversationItem item, NativeLocalization? locale = null)
    {
        if (item.Role != "acp_tool") return [];
        string L(string zh, string en) => locale?.Choose(zh, en) ?? zh;
        var sections = new List<NativeToolSection>();
        if (!string.IsNullOrWhiteSpace(item.Locations))
        {
            try
            {
                if (JsonNode.Parse(item.Locations) is JsonArray locations)
                    foreach (var row in locations.Take(128))
                    {
                        var path = Text(row?["path"]);
                        if (path.Length > 0) sections.Add(new(L("位置", "Location"), path + (row?["line"] is { } line ? ":" + line : "")));
                    }
                else sections.Add(new(L("位置", "Location"), item.Locations));
            }
            catch { sections.Add(new(L("位置", "Location"), item.Locations)); }
        }
        try
        {
            if (JsonNode.Parse(item.Text) is not JsonArray blocks) return sections.Append(new(L("输出", "Output"), item.Text)).ToArray();
            foreach (var block in blocks.Take(128))
            {
                var kind = Text(block?["type"]);
                if (kind == "diff")
                {
                    sections.Add(new(L("文件差异 · ", "File changes · ") + Text(block?["path"]), L("修改前\n", "Before\n") + Text(block?["oldText"]) + L("\n\n修改后\n", "\n\nAfter\n") + Text(block?["newText"])));
                }
                else if (kind == "terminal") sections.Add(new(L("终端", "Terminal"), Text(block?["terminalId"])));
                else if (kind == "content" && block?["content"] is JsonObject content)
                {
                    if (Text(content["type"]) == "text") sections.Add(new(L("输出", "Output"), Text(content["text"])));
                    else sections.Add(new(L("资源", "Resource"), Text(content["resource"]?["uri"]) is { Length: > 0 } uri ? uri : content.ToJsonString()));
                }
                else if (block != null) sections.Add(new(L("输出", "Output"), block.ToJsonString()));
            }
        }
        catch { sections.Add(new(L("输出", "Output"), item.Text)); }
        return sections.Where(section => !string.IsNullOrWhiteSpace(section.Text)).ToArray();
    }
    private static string Text(JsonNode? value) => value is JsonValue scalar && scalar.TryGetValue<string>(out var text) ? text : "";
}
