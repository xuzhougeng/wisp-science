using System.Text.Json;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed record NativeDocumentSource(string ProjectId, string Path, NativeDocumentLocation Location);

public sealed record NativeComposerQuote(string SessionId, int UserIndex, string Role, string Text, NativeDocumentSource? Document = null)
{
    public string Source => Document is { } source
        ? $"文件 {source.Path} · {source.Location.Label} · 项目 {source.ProjectId}"
        : $"会话 {SessionId} · 第 {UserIndex + 1} 轮 · {Role}";
    public string Message => $"引用来源：{Source}\n" + string.Join("\n", Text.Replace("\r\n", "\n").Replace('\r', '\n').Split('\n').Select(line => "> " + line));
    public static NativeComposerQuote? From(ConversationSnapshot snapshot, int index, string text)
    {
        if (index < 0 || index >= snapshot.Items.Length || string.IsNullOrWhiteSpace(text)) return null;
        var user = (snapshot.UserOffset ?? 0) - 1;
        for (var n = 0; n <= index; n++) if (snapshot.Items[n].Role == "user") user++;
        return user < 0 ? null : new(snapshot.SessionId, user, snapshot.Items[index].Role, text.Trim());
    }
    public static string Compose(string draft, NativeComposerQuote[] quotes) =>
        string.Join("\n\n", quotes.Select(quote => quote.Message).Append(draft.Trim()).Where(value => value.Length > 0));
}

/// <summary>Uses persisted context totals, not cumulative billing tokens. Compaction
/// invalidates the old bucket split until a fresh usage row arrives.</summary>
public sealed record NativeContextUsage(ulong Used, ulong Max, IReadOnlyList<(string Label, ulong Tokens)> Rows)
{
    public string Label => Max == 0 ? "上下文 · 窗口未知" : $"上下文 · {Math.Round(100d * Used / Max):0}%";
    public string CompactLabel => Max == 0 ? "?" : $"{Math.Round(100d * Used / Max):0}%";
    public string Total => Max == 0 ? $"约 {Used:N0} Tokens · 窗口未知" : $"约 {Used:N0} / {Max:N0} Tokens";
    public string Tone => Max > 0 && (double)Used / Max > .9 ? "clay-strong" : "text-muted";
    public static NativeContextUsage? Latest(IEnumerable<ConversationItem> items)
    {
        ulong? compacted = null;
        foreach (var item in items.Reverse())
        {
            if (item.Role is not ("usage" or "compaction")) continue;
            try
            {
                using var document = JsonDocument.Parse(item.Text);
                var root = document.RootElement;
                if (root.ValueKind != JsonValueKind.Object) continue;
                if (item.Role == "compaction")
                {
                    if (compacted == null && root.TryGetProperty("undone", out var undone) && undone.ValueKind == JsonValueKind.True) continue;
                    if (compacted == null && (!root.TryGetProperty("strategy", out var strategy) || strategy.GetString() != "auto_continue")
                        && root.TryGetProperty("after", out var after) && after.TryGetUInt64(out var value)) compacted = value;
                    continue;
                }
                static ulong Number(JsonElement node, string key) => node.ValueKind == JsonValueKind.Object
                    && node.TryGetProperty(key, out var value) && value.TryGetUInt64(out var number) ? number : 0;
                var used = Number(root, "ctx_tokens"); var max = Number(root, "max_context");
                if (used == 0 && max == 0) continue;
                var rows = new List<(string Label, ulong Tokens)>();
                if (compacted == null && root.TryGetProperty("context_usage", out var buckets))
                    foreach (var (key, label) in new[] { ("system_prompt", "系统提示词"), ("tool_definitions", "工具定义"),
                        ("rules", "规则"), ("skills", "技能"), ("mcp_dynamic_tools", "MCP 与动态工具"),
                        ("subagent_definitions", "子智能体定义"), ("conversation", "对话") })
                        if (Number(buckets, key) is > 0 and var tokens) rows.Add((label, tokens));
                if (rows.Count == 0) rows.Add(("对话", compacted ?? used));
                return new(compacted ?? used, max, rows);
            }
            catch (Exception ex) when (ex is JsonException or InvalidOperationException or FormatException) { }
        }
        return null;
    }
}
