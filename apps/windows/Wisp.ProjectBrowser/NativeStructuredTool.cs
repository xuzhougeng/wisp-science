using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed record NativeToolSection(string Label, string Text);
public static class NativeStructuredTool
{
    public static NativeToolSection[] Sections(ConversationItem item)
    {
        if (item.Role != "acp_tool") return [];
        var sections = new List<NativeToolSection>();
        if (!string.IsNullOrWhiteSpace(item.Locations))
        {
            try
            {
                if (JsonNode.Parse(item.Locations) is JsonArray locations)
                    foreach (var row in locations.Take(128))
                    {
                        var path = Text(row?["path"]);
                        if (path.Length > 0) sections.Add(new("位置", path + (row?["line"] is { } line ? ":" + line : "")));
                    }
                else sections.Add(new("位置", item.Locations));
            }
            catch { sections.Add(new("位置", item.Locations)); }
        }
        try
        {
            if (JsonNode.Parse(item.Text) is not JsonArray blocks) return sections.Append(new("输出", item.Text)).ToArray();
            foreach (var block in blocks.Take(128))
            {
                var kind = Text(block?["type"]);
                if (kind == "diff")
                {
                    sections.Add(new("文件差异 · " + Text(block?["path"]), "修改前\n" + Text(block?["oldText"]) + "\n\n修改后\n" + Text(block?["newText"])));
                }
                else if (kind == "terminal") sections.Add(new("终端", Text(block?["terminalId"])));
                else if (kind == "content" && block?["content"] is JsonObject content)
                {
                    if (Text(content["type"]) == "text") sections.Add(new("输出", Text(content["text"])));
                    else sections.Add(new("资源", Text(content["resource"]?["uri"]) is { Length: > 0 } uri ? uri : content.ToJsonString()));
                }
                else if (block != null) sections.Add(new("输出", block.ToJsonString()));
            }
        }
        catch { sections.Add(new("输出", item.Text)); }
        return sections.Where(section => !string.IsNullOrWhiteSpace(section.Text)).ToArray();
    }
    private static string Text(JsonNode? value) => value is JsonValue scalar && scalar.TryGetValue<string>(out var text) ? text : "";
}
